# 0129. Specification mutation testing and trivial-contract detection

- Status: Proposed
- Date: 2026-10-31
- Deciders:
- Resolves: issue #1293
- Related:
  - ADR 0127 (Una Quality foundation) and `spec/32` (Draft): §8 (mutation outcomes and the two indicators), §9 (`contract.signals`), §14 (configuration), §18 (codes); this record defines the engine phase 3 (#1299) shares and the checks behind UQ5201;
  - ADR 0128 (the guarantee diff), which reports this record's results as deltas and never gates on them;
  - issue #1289 (requirement coverage), which proves traceability only; this record asks whether the traced specification is strong enough;
  - `spec/08` (requirements and tests), ADR 0105 (the `test` declaration);
  - `spec/12` (verification): the fragment (§2), obligations of tests' `do` and `expect` lines (§3), statuses (§6), the step budget and seed (§9);
  - `spec/13 §6`, whose `trivial_ensures` is the first trivial-contract check;
  - ADR 0112 (check elision), for the run-time checks a specification can still cause;
  - ADR 0092 (conditions on operations and `[contracts]` locks).
- Target: the toolchain. No construct of the language changes, no diagnostic code `U....` is added, and no edition changes.
- Amends, on acceptance and with the slices that implement it (section H): `spec/32 §8` (the engine, the operators, the outcomes, `spec.sensitivity`), `§9` (the trivial-contract checks of `contract.signals`), `§14.2` (the `[mutation]` table and reviewed equivalent mutants), `§18` and `§19` (codes UQ4102 and UQ5401, rule 19).

> **In plain words.**
> - **What it asks.** Is the specification strong enough? A proof only says that the code meets its contract; a contract that says little is easy to meet.
> - **How.** It makes small wrong versions of the code and checks whether tests or the verifier notice. It also makes weaker versions of the contracts and checks whether anything independent notices that the promise shrank.
> - **What it reports.** Two separate numbers with their denominators: the share of wrong code the tests catch, and the share the verifier rejects. Plus findings for contracts that say nothing: a precondition no call can meet, a postcondition that is always true, a result nothing constrains, a contract a constant meets.
> - **What it is not.** A proof that the specification matches what anyone intended. No fixed score is required.

## Questions for acceptance

Four points rest on assumptions that acceptance confirms or changes:

1. **Operators apply to the surface tree (rule 3).** Mutants are edits of the source as written, so each one prints as a one-line source change and goes through the whole pipeline, desugaring included. The alternative, mutating Core IR, is cheaper per mutant but gives mutants no source text shows; recorded under Alternatives.
2. **Specification mutants are judged by the verifier, and run tests only where the specification has a run-time effect (rule 9).** The code of a specification mutant is unchanged, so a test run can differ only through a run-time check that remains after elision (ADR 0112). The independent evidence is therefore static: callers' obligations, tests' verified `expect` lines, locks and requirement-linked contracts.
3. **`no_coverage` stays (rule 6).** The issue lists five outcomes; `spec/32 §8` also has `no_coverage`. This record keeps it as a refinement of `survived` (a survivor no test evaluates), counted as undetected, so the formula of `spec/32 §8` is unchanged.
4. **Reviewed equivalent mutants are recorded in `quality.toml` (rule 12).** A person's judgment that a mutant is equivalent must be stored somewhere reviewed and bound to the code it judged. This record puts it beside waivers, with the same binding to the declaration's fingerprint.

## Glossary

- **Mutant:** a copy of the package, in memory, with one change made by one operator at one site.
- **Code mutant:** a mutant whose change is in a function's body; its specification is unchanged.
- **Specification mutant:** a mutant whose change makes a contract, refinement or condition weaker; its code is unchanged.
- **Outcome:** what happened to a mutant (rule 6).
- **Independent evidence:** a check that does not consist of the mutated declaration proving its own mutated contract: another declaration's obligations, a test's obligations or run, a lock, a requirement's derived contracts.
- **Equivalent mutant:** a mutant with the same behavior as the original on every input the contract admits, so no test can kill it.

## Context

`una check` proves that code meets its contract. It cannot say whether the contract is worth meeting. `ensures result >= 0` on a function that should return a sorted copy is proven and nearly empty; `requires amount > 0 and amount < 0` makes every obligation of the body hold, since nothing can call it. #1289 shows that each requirement has a contract or test; it does not show that the contract is strong. Agents make this worse: an agent that cannot prove a strong postcondition can weaken it until the proof passes.

Mutation testing is the established way to measure what a test suite checks: make small faults and count how many the tests detect. The sources the issue and the metric definitions cite, and what was verified of them:
- the Stryker documentation (FAQ, fetched): the mutation score is detected over detected plus undetected, with timeouts counted as detected and survivors and uncovered mutants as undetected; invalid mutants are excluded. It does not define equivalent mutants;
- Inozemtseva and Holmes, ICSE 2014 (confirmed by search, not fully read): coverage alone predicts test effectiveness only weakly once suite size is controlled;
- Just et al., FSE 2014 (confirmed by search, not fully read): mutant detection correlates with real-fault detection independently of coverage, with a coupling effect for 73% of 357 real faults studied.
These motivate reporting mutation next to coverage; this record claims nothing more from them.

Una has what other languages lack: a verifier. A mutant can be rejected by `una check` before any test runs. That rejection is evidence about the specification and the verifier, not about the tests, and adding it to the test score would overstate both. And Una can mutate the specification itself, then ask whether anything independent notices that the promise shrank.

The forces:
- **Two kinds of evidence must not be merged.** Tests and the verifier detect different things; one number for both hides which one is weak.
- **Denominators move.** Excluding equivalent, invalid or timed-out mutants changes the percentage; without raw counts a better score may only mean a smaller denominator.
- **Equivalent mutants are undecidable in general.** A tool that guesses them away inflates the score; a tool that counts them all deflates it. Only a proof or a person can remove one.
- **The solver decides some trivial-contract questions exactly,** within `spec/12 §2`, and others not at all. A check must say which.
- **Cost.** One mutant can cost a full check and test run; a package has thousands of sites. The run must be bounded and still reproducible.

### Departures from the issue

- **"Start as an external evaluation command"** is `una quality mutants`, a subcommand of the quality tool that never runs inside `una check` and is never required by a gate unless the project's `quality.toml` says so (rule 15).
- **Trivial-contract checks** run in `una quality --mode verified`, not only in the external command, because they cost one query per clause and need no mutant.

## Decision

### A. The engine

1. **One engine, two families.** The mutation engine of Una Quality phase 3 (#1299) makes both code mutants and specification mutants, with one mutant record, one outcome set, one budget and one seed. `una quality --mode full` runs code mutants as part of the quality report (`spec/32 §8`); `una quality mutants` runs either family on request (rule 15).
2. **Mutants live in memory.** A mutant is the package's sources with one edit, held in memory; the sources on disk are never written (ADR 0127 rule 31). Each mutant is checked and tested as `una check` and `una test` would, without host grants and with the virtual time of `una test`.
3. **Operators apply to the surface tree** (Question 1). Each operator is a rewrite of one node of the surface syntax tree into another that parses; the mutant then goes through formatting-independent elaboration, desugaring, checking and tests. Each mutant prints as its source span and a one-line replacement.
4. **The mutant id.** `mu-` and the first 12 hex digits of SHA-256 over the operator id and version, the declaration's qualified name, the ordinal of the site in the declaration's surface tree in source order, and the replacement text. The id survives edits of other declarations, and a reviewed decision (rule 12) is bound to it and to the declaration's fingerprint.

### B. Operators

5. **Code operators** (the body changes, the contract does not). Each has an id and a version; the first set:

| Id | Change | Example |
| --- | --- | --- |
| `rel.boundary` | `<` and `<=`, `>` and `>=` exchanged | `if n < limit` to `if n <= limit` |
| `rel.negate` | `==` and `!=`, a comparison replaced by its negation | `a == b` to `a != b` |
| `cond.drop_conjunct` | one operand of `and` removed; one operand of `or` removed | `ok and n <= max` to `ok` (drops an upper bound) |
| `cond.force` | a condition replaced by `true` or by `false` | `if is_admin(user)` to `if true` (removes an authorization test) |
| `arith.swap` | `+` and `-`, `*` and `div` exchanged where the types allow | `total + fee` to `total - fee` |
| `const.shift` | an integer literal `k` replaced by `k + 1`, `k - 1` and `0` | `limit = 100` to `limit = 101` |
| `unit.swap` | a unit of measure replaced by another of the same dimension (`spec/25`) | `ms` to `s` |
| `update.skip` | a `set`, an assignment to an `edit` parameter, or an operation call whose result is `Unit`, removed | `set stock.count = n` removed |
| `fail.swap` | `fail V` replaced by `fail W`, another variant of the same kind | `fail NotFound` to `fail Denied` |
| `handle.pass` | the body of an `if error` clause replaced by `pass` where the types allow | |
| `when.arm` | the body of one `when` arm replaced by another arm's body of the same type | |

   An operator never produces a mutant that adds authority, a `trusted` module, `checked` or a policy call: such a mutant would test the checker's rejection of authority, which `una check` already decides.
6. **Specification operators** (the contract becomes weaker, the code does not):

| Id | Change |
| --- | --- |
| `spec.drop_ensures` | one `ensures` clause removed |
| `spec.weaken_ensures` | a conjunct of an `ensures` removed; a bound loosened (`<` to `<=`, a literal bound moved by one away from the constraint) |
| `spec.relax_requires` | one `requires` clause removed, or a conjunct of it, or a bound loosened |
| `spec.drop_refinement` | a parameter's or the result's refined kind replaced by its base type, or one `where` of an inline refinement removed |
| `spec.raise_bound` | a literal bound of a refinement or clause moved by one, and to the type's limit, in the permissive direction |
| `spec.drop_condition` | a `where` condition on an operation in `uses` removed (ADR 0092) |
| `spec.loosen_result` | an `ensures` that equates `result` to a term replaced by a bound on it (`==` to `<=` or `>=`) |

   Each specification mutant is weaker than the original by construction; the engine does not need to prove it, and the guarantee diff (ADR 0128) would report it as a regression.

### C. Outcomes and indicators

7. **Order of evaluation.** For each mutant, in order, stopping at the first that applies:
   1. tier 0 rejects it (it does not type, a name is missing): `invalid`;
   2. tier 1 reports an obligation `refuted` or `unknown` with a reason other than a budget, or U3023 for a lock: `rejected_by_verifier`, with the obligations named and the declaration that holds each (rule 10);
   3. the verifier reached its step budget or wall-clock guard: `timeout`, phase `verifier`;
   4. a code mutant whose site no test evaluates: `survived` with the refinement `no_coverage` (Question 3), without running the tests;
   5. the tests run: a test fails or aborts, `killed_by_tests`; a test exceeds its budget (rule 17), `timeout`, phase `tests`; every test passes, `survived`.
8. **Two indicators, never merged.** For code mutants:
   - **test mutation score** `mutation.test_score` = detected / (detected + undetected), with detected = `killed_by_tests` + `timeout` (phase `tests`), and undetected = `survived` (`no_coverage` included). Its denominator holds only the mutants the verifier let through to the tests;
   - **verifier rejection rate** `mutation.verifier_rejection_rate` = `rejected_by_verifier` / valid, with valid = every mutant that is not `invalid`. A `timeout` of phase `verifier` stays in the denominator and not in the numerator, so the solver's limits never raise the rate;
   - neither is ever added to the other or shown as one number. Every report prints, next to each ratio, its numerator, its denominator and the raw count of every outcome, `invalid`, `timeout` by phase, proven-equivalent and reviewed-equivalent included, so a change of denominator is visible;
   - an empty denominator gives `not_applicable` (`spec/32 §2`).
9. **Specification mutants and `spec.sensitivity`** (Question 2). A specification mutant's code is unchanged, so:
   - the tests are run only for a mutant that changes a run-time check remaining after elision (ADR 0112); otherwise steps 4 and 5 of rule 7 are skipped and the outcome is decided by the verifier;
   - a mutant the mutated declaration alone rejects (its own body no longer proves, as when a relaxed `requires` was needed) is `rejected_by_verifier` with evidence `self`;
   - **`spec.sensitivity`** = mutants noticed by independent evidence / valid specification mutants, where the independent evidence is `caller` (another declaration's obligation), `test` (an obligation of a test's `do` or `expect` line, `spec/12 §3`, or a failed run), `lock` (U3023, ADR 0092) or `requirement` (a contract derived from the same requirement). Evidence `self` is counted and shown apart and is not in the numerator: a function that satisfies its weakened contract proves nothing by itself;
   - a surviving specification mutant names an **unobserved clause**: nothing outside the function relies on it, and no test checks it;
   - the report gives `spec.sensitivity` per contract, per requirement (through `derived from`) and per package, with raw counts. It is never presented as evidence that the specification matches intent.
10. **Evidence is named.** A `rejected_by_verifier` or `killed_by_tests` outcome lists what noticed it: the obligations by id and kind with their declarations, or the tests by module and name. The report groups survivors per contract and per requirement, which is where a weak specification shows.

### D. Equivalent mutants

11. **Proven equivalence.** For a code mutant whose changed expression and its original are both in the refinement language, the engine asks the solver whether they can differ under the site's path context: `Π ∧ e ≠ e′` (`spec/12 §3`). `unsat` proves the mutant equivalent: it is `equivalent` (label `exact`), excluded from both denominators, and listed with its count. `sat` or `unknown` proves nothing, and the mutant is evaluated as rule 7 says. One query per such mutant, cached like obligations.
12. **Reviewed equivalence** (Question 4). A survivor that a person judges equivalent is recorded in `quality.toml`:

    ```toml
    [[mutation.equivalent]]
    mutant = "mu-7c01d2e9aa40"
    symbol = "stock.restock"
    fingerprint = "sha256:..."
    reason = "both bounds are unreachable under the requires"
    reviewer = "inventory maintainers"
    ```

    It is excluded from the denominators and listed with its reason, as a waiver is (`spec/32 §14.4`): it lapses when the declaration's fingerprint changes, a malformed entry is UQ0001, and a new entry against the baseline is reported by the evolution gate (UQ9301). Until a person records it, a survivor counts as `survived`.
13. **Suspected equivalence.** The engine flags survivors it suspects (for example a `const.shift` on a value used only in a comparison that the shift does not cross) as `mutation.equivalent_suspects`, label `heuristic`, for review. A suspect is never excluded and never scored.

### E. Trivial-contract checks

14. **The checks,** each with its label and how it is decided. They feed `contract.signals` (`spec/32 §9`, UQ5201), one finding per clause or contract, severity `warn` in every preset, never a silent pass:

    | Check | Decided by | Label |
    | --- | --- | --- |
    | `requires` unsatisfiable: `Γ ∧ P` has no model, so no call can meet it | solver, in the fragment | exact |
    | `ensures` valid with no hypothesis (`spec/13 §6` `trivial_ensures`) | solver | exact |
    | `ensures` implied by the precondition alone: `Γ ∧ P ⇒ E` with `result` and outputs unconstrained, so it says nothing about what the function computes | solver | exact |
    | a clause that holds vacuously: an `ensures A implies B` (or `when` guard) whose `A` is unsatisfiable under `Γ ∧ P` | solver | exact |
    | no `ensures` and no result refinement constrains `result`, nor the final value of any `edit` parameter: the contract omits every output | syntax | exact |
    | met by a constant: for each candidate `c` of the result type (0, 1, -1, the bounds of the result's refinement, `true`, `false`, the empty text and list, each variant without fields), `Γ ∧ P ⇒ Q[result := c]` holds | solver, one query per candidate | exact when found; absence is not claimed |
    | contracts tied only to positive tests: a function that declares `fails` and whose every test expects success | syntax over tests | heuristic |
    | a requirement whose derived functions have no `ensures` beyond trivial ones and no test | syntax | exact |

    - `Γ` and `P`, `Q` are the context and effective conditions of ADR 0128 rule 8.
    - A query outside the fragment, or one the solver answers `unknown` or out of budget, gives the finding with status `inconclusive` and the marker `review_required`: the check does not decide, and the clause is not reported as passing.
    - The "met by a constant" check finds a witness or nothing: finding none never claims the contract excludes constants.
    - Cost: at most one query per clause for each solver check, and a bounded number per contract for constants (the candidate list is fixed by the result type, at most a dozen); cached like obligations.

### F. Commands, configuration and reporting

15. **The command.** `una quality mutants [path]` runs the engine on request: `--family code|spec|both` (default `both`), `--seed <n>` (default 0), `--max-mutants <n>`, `--symbol <qualified.name>` (one declaration and the declarations that call it), and `--json`. It prints the outcomes, the indicators of rules 8 and 9, the survivors grouped by contract and requirement, the equivalent mutants, and the seed and budget used. Its exit code is 0 unless a usage, configuration or internal error occurs: it is an evaluation, not a gate. Within `una quality --gate`, mutation results gate only as the project's `quality.toml` sets UQ4101, UQ4102 and UQ5401 (rule 19).
16. **Configuration.** `quality.toml` gains a `[mutation]` table: `operators` (the ids of rules 5 and 6 to use, all by default), `families`, `seed`, `max_mutants`, `test_budget_factor` (rule 17), and the `[[mutation.equivalent]]` entries of rule 12. `[limits] mutants` of `spec/32 §14.2` remains the hard cap.
17. **Cost and determinism.**
    - **Selection.** When a run's mutants exceed `max_mutants`, the engine orders them by SHA-256 of the seed and the mutant id and takes the first ones: a deterministic, seeded sample that does not change when unrelated declarations change. The report states the sample's size, the population's size and the seed.
    - **Budgets in steps, not seconds.** The tests of a mutant run with a budget of interpreter steps, `test_budget_factor` (default 10) times the steps the original suite took; the verifier uses the lock's step budget (`spec/12 §9`). Both are deterministic. The wall-clock guard of `spec/12 §5` stays as a backstop; a result it ends is `timeout` and marked non-deterministic.
    - **Incremental work.** A mutant re-checks the mutated declaration and the declarations whose obligations depend on its contract; every other query is a proof-cache hit (`spec/12 §9`), since its text is unchanged. Tests run only those that reach the mutated declaration.
    - **Order.** Mutants MAY be evaluated in parallel; results are reported in the order of the mutant ids' declarations and sites, so output bytes are the same.
    - **Complexity.** Generating mutants is O(n) in the package's surface nodes; each mutant costs at most one incremental check and the tests that reach it; a run is bounded by `max_mutants` and the step budgets.
18. **Where results go.** The code family feeds `mutation.*` of `spec/32 §8`; the specification family feeds `spec.sensitivity`; the trivial-contract checks feed `contract.signals` (`spec/32 §9`). The guarantee diff (ADR 0128 rule 20) reports their changes against the baseline, never as a gate. All of it is data in `una.quality/1`, with statuses of `spec/32 §2`.
19. **Codes.** Added to `spec/32 §18` on acceptance, structural gate: **UQ4102** "verifier rejection rate below its limit" and **UQ5401** "specification sensitivity below its limit". UQ4101 (test mutation score) and UQ5201 (a contract that says less than it seems to) keep their meaning; UQ5201's finding names the check of rule 14 as its metric (`contract.signals.unsatisfiable_requires`, `contract.signals.vacuous_ensures`, and so on). No preset sets UQ4102 or UQ5401 above `info` until calibration on Una's own corpus gives reference values.

### G. Out of scope

20. Not part of this record: a fixed score any package must reach; a claim that a high sensitivity shows the specification matches intent; mutants of the standard library, the runtime or dependencies; higher-order mutants (two changes at once); mutants made by a model; changes to the language.

## Complexity

Stated in rule 17: O(n) generation; per mutant, one incremental check and the tests that reach the mutated declaration; bounded by `max_mutants` and step budgets. Trivial-contract checks: one query per clause per solver check, a fixed number per contract for constants. No pass is quadratic in the package; scaling tests at growing package sizes come with slice 1.

## Security

Mutants are made and evaluated in memory, with no host grant and no file write (ADR 0127 rule 31). No operator adds authority, `trusted`, `checked` or a policy call (rule 5). Reviewed equivalences are bound to a fingerprint and listed in every report, so they cannot silently shrink a denominator. A mutation run never changes what `una check` accepts.

## Verification

No obligation kind is added. Equivalence queries (rule 11) and trivial-contract queries (rule 14) are solver queries in the encoding of `spec/12 §4`, cached like obligations; `unknown` is never a decision.

## Information flow

Mutants are checked for information flow like any code. A mutant that `una check` rejects for a flow is `rejected_by_verifier`, evidence `flow`. Reports hold outcomes and names, never values of a run.

## Diagnostics

No `U` code is added. UQ4102 and UQ5401 join the `UQ` registry with explain texts.

## Consequences

- A reviewer sees which contracts nothing relies on and which tests check nothing, per contract and per requirement.
- The verifier's contribution and the tests' contribution are reported apart, with denominators, so neither hides the other.
- Contracts that say nothing are found by exact solver checks where the fragment allows and marked for review elsewhere.
- Mutation costs time; it runs on request, in `full` mode, or with the release's heavy evidence (ADR 0127 rule 34, phase 3), within budgets.
- Calibration is needed before any preset gates on the new indicators.
- Deferred: higher-order mutants, operators for edition `0.4` concurrency forms, mutants of dependencies.

## Alternatives considered

- **Mutate Core IR** (Question 1). Cheaper per mutant and uniform after desugaring, but a mutant would have no source text to show, and some Core mutants correspond to no program a person could write. Rejected for the first version.
- **One combined score of tests and verifier.** Rejected: the issue and `spec/32 §8` forbid it; it hides which evidence is weak.
- **Exclude suspected equivalent mutants automatically.** Rejected: it inflates the score by a guess. Only a proof (rule 11) or a recorded review (rule 12) excludes one.
- **Run the tests on every specification mutant.** Rejected as waste: the code is unchanged, so only run-time checks can differ, and rule 9 runs those.
- **A required minimum score.** Rejected by the issue: no fixed score is required; the project's `quality.toml` decides.
- **Decide "met by a constant" with a quantified query.** `∃c ∀x` is outside the quantifier-free fragment of `spec/12 §2`. Rejected in favor of fixed candidates, each a ground query.

## Implementation plan

### H. Slices

Each slice is its own card and pull request; each amends `spec/32` with fixtures of known values.

1. **Trivial-contract checks** (#1320). Rule 14 in `una quality --mode verified`, feeding UQ5201; the false-positive rate measured on the example packages and recorded.
2. **The mutation engine and code operators** (#1321). Rules 2 to 5, 7, 8, 10, 11 and 17: in-memory mutants, ids, the outcome order, the two indicators, proven equivalence, seeded sampling and step budgets. Shared with phase 3 (#1299).
3. **Specification mutants and sensitivity** (#1322). Rules 6 and 9: specification operators, evidence sources, `spec.sensitivity` per contract and requirement, unobserved clauses.
4. **Command, configuration and review** (#1323). Rules 12, 13, 15, 16 and 19: `una quality mutants`, the `[mutation]` table, reviewed and suspected equivalence, codes UQ4102 and UQ5401 with explain texts.

## Conformance

- **Seeded weak specifications are flagged, strong ones are not** (the issue's acceptance): fixtures with an unsatisfiable `requires`, a tautological `ensures`, an `ensures` implied by the precondition, a vacuous implication, a contract with no output constrained and a contract met by `0` each give their UQ5201 finding; a fixture of strong contracts (each clause relied on by a caller or a test) gives none, and every specification mutant of it is noticed by independent evidence.
- **Indicators:** a fixture whose mutants are all rejected by the verifier gives a verifier rejection rate of 100% and a test mutation score of `not_applicable`, never 100%; the raw counts print beside both.
- **Equivalence:** a mutant proven equivalent leaves both denominators and is listed; a suspected one stays counted.
- **Determinism:** two runs with the same seed give identical bytes; a different seed with a sample cap gives a different, reproducible sample.
- **False positives:** the trivial-contract checks run on every example package; their findings are reviewed and the false-positive rate is recorded with slice 1.
- **No language change:** the checks and the engine pass the existing conformance suite unchanged.
