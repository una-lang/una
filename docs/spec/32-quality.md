# 32. Quality

Status: Draft (ADR 0127, accepted on 2026-11-02 with ADRs 0128, 0129 and 0131, whose decisions of acceptance this text follows). Each section becomes normative with the phase that implements it (ADR 0127 rule 34), with its fixtures; until then nothing in this chapter is normative.

(v0.5, ADR 0137.) **Binding words.** In edition `0.5`, a binding of the surface language that this chapter writes `let` is written `const`, and one it writes `let edit` is written `var` (`spec/02 §6`, `spec/17 §2`); the rules are unchanged: rule 3 of the matching of functions (§3) reads "a contiguous run of `const` and `var` statements"; its table of costs and its operators name Core forms and stay. A Core `let` (`spec/09`) stays. The chapter's Una blocks keep the words of edition `0.4` until issue #1630 moves those of edition `0.5`.

Not implemented in release 0.4.0: `una` has no `quality` and no `diff` command, and none of the reports, rules or gates of this chapter.

Una Quality measures a package and reports the values, the findings of the rules a project configures, and the state of three gates. It is a tool of the toolchain, `una quality`; it adds no syntax, no construct and no `U` diagnostic code, and it never changes what `una check` accepts.

**Editions.** Una Quality reads packages of every edition. The definitions for constructs of editions `0.4` and `0.5` are marked *preview*: they follow the terms of a preview edition (ADR 0121, as its amendment of 2026-11-06 extends them) and may change by a new metric version.

## 1. Terms

1. A **metric** is a named measurement with a written algorithm and a version, written `id@version`, such as `complexity.cyclomatic@1`. It has a unit of analysis and gives one value per unit.
2. A **unit of analysis** is a function, a module, a package or a file. Handlers' operations are functions. A lambda is measured as its own unit, `m.f#k` (its ordinal `k` in source order within `m.f`), and its contributions also count for its enclosing function.
3. An **observation** is one value of one metric for one unit, with a status (§2), a label (§2), a location and, where the metric has them, its numerator and denominator.
4. A **rule** is a condition on a metric's value or on its change against a baseline, with a severity (§14) and a gate (§15).
5. A **finding** is an observation that crosses a rule. It has a `UQ` code (§18).
6. **Evidence** is a fact a metric reads beyond the source: obligation statuses, test outcomes, coverage counts, mutation outcomes. Evidence carries the digest of the inputs it was computed from.
7. A **gate** is a set of rules: `assurance`, `structural` or `evolution` (§15).
8. A **snapshot** is the record of every observation of a run (§17.2). A **baseline** is the snapshot a run is compared with.
9. A **waiver** is a recorded, expiring exception for one rule on one symbol (§14.4). An **approval** is a recorded, expiring acceptance of one guarantee change, an entry of the guarantee diff's approvals file (§14.4 rule 4, ADR 0128 rule 22); a waiver is never an approval.
10. A **preset** is a named, versioned set of default severities and limits (§14.3).
11. A **mode** is `fast`, `verified` or `full` (§2 rule 7).

## 2. Values

1. **Status.** Every observation has exactly one status:
   - `complete`: measured over all of its unit;
   - `partial`: measured over part of its unit (a declaration with an unknown Core form, a module with rejected declarations);
   - `not_measured`: not measured (rule off, mode too low, stale evidence);
   - `not_applicable`: the metric has no meaning for the unit, such as a ratio whose denominator is 0;
   - `inconclusive`: measured, with evidence that disagrees or a solver answer of `unknown`;
   - `timeout`: a budget ran out.
2. **Reason.** A status other than `complete` has a reason: `off`, `mode`, `stale`, `budget`, `unknown_construct`, `check_failed`, `edition`, `solver` or `disagreement`.
3. **Never passing when not measured.** A value whose status is not `complete` MUST NOT be reported as 0, as 100% or as passing a rule. A ratio whose denominator is 0 MUST be `not_applicable`.
4. **Label.** Every observation is `exact`, a deterministic count or ratio from the written algorithm, or `heuristic`, an estimate or a signal.
5. **Determinism.** The same sources, edition, tool version, configuration, baseline, mode and date MUST give the same output bytes. Iteration follows source order or sorted keys. No model and no randomness are used; mutation uses a seed recorded in the report.
6. **Aggregation.** A per-function metric is aggregated per module and per package as its maximum, its 95th percentile (nearest rank), its count above each limit, and its sum. A mean MUST NOT be reported alone.
7. **Modes.** `fast` reads the surface tree and Core IR at tier 0. `verified` adds the facts of `una check` at tier 1. `full` adds test runs, coverage and mutation. A metric whose definition names a higher mode than the run's is `not_measured` with reason `mode`.
8. **Scope.** `all`, `changed` (files whose digest differs from the baseline's) or `new` (new or changed symbols, matched by §3 rule 4). The scopes `tests` (examples and tests), `generated` (files the configuration lists, with a reason) and `src` are reported apart. No file of the package is excluded silently.
9. **Versions.** The report carries the tool version, each metric's version, each preset's version and the configuration digest.

## 3. Representations and identity

1. **Core IR** (`spec/09`, `una.core/1` of `spec/13 §8`) is read for the metrics marked *Core* below: decisions, the call graph, dependencies, Halstead and branch coverage.
2. **The surface tree** (`spec/02`) is read for the metrics marked *surface*: cognitive complexity, nesting, expression depth, lines and duplication.
3. **Spans.** Every observation and contributor maps to a source span, `file`, `start` and `end` as `[line, column]`, 1-based. A Core node without a span takes the span of the innermost enclosing node with one, and the contributor records `"span": "enclosing"`.
4. **Symbol identity.** A symbol is its qualified name; a lambda is `m.f#k`. Matching against a baseline:
   1. equal qualified names match;
   2. among unmatched symbols, a pair whose body fingerprints are equal, and unique on each side, match as `renamed`. The fingerprint is the SHA-256 of the declaration's body in Core IR, in the encoding of `una.core/1`, with every binder name replaced by its first-occurrence ordinal and every span removed. The header, its contract included, is outside the hash (ADR 0127 rule 5): a function renamed with a changed contract still matches as `renamed`, and the guarantee diff compares its contract (ADR 0128 rule 7);
   3. among the rest, a new function whose fingerprint equals that of a contiguous run of `let`s removed from a matched function is `extracted_from` it, labeled `heuristic`;
   4. the rest are `new` or `removed`. Equal fingerprints shared by more than one symbol on a side match none of them.

## 4. Size

Representation: surface. Cost: O(n) in the file's characters.

| Id | Definition | Unit |
| --- | --- | --- |
| `size.lines` | Physical lines of the file. A last line without a newline counts. | file |
| `size.sloc` | Lines with at least one character that is not whitespace and not part of a comment. | file, module |
| `size.comment_lines` | Lines whose only non-whitespace content is a `--` or `---` comment. | file, module |
| `size.function_sloc` | `size.sloc` from the first line of a function's header to the last line of its body, minus its contract lines. | function |
| `size.contract_sloc` | Lines of a function's `requires`, `ensures`, `uses`, `fails` and `derived from` clauses. | function |
| `size.function_total_sloc` | `size.function_sloc + size.contract_sloc`. | function |
| `size.parameters` | Declared parameters, labeled or not. `uses` bindings are not parameters. | function |
| `size.public_items` | `pub` functions, kinds, predicates, capabilities, handlers and policies. | module |
| `size.kinds_per_module` | `kind` declarations. | module |
| `size.comment_ratio` | `comment_lines / (sloc + comment_lines)`; `not_applicable` when both are 0. | file, module |

A line holding code and a trailing comment is a source line. A line inside a multi-line text literal is a source line.

## 5. Complexity

### 5.1. `complexity.cyclomatic@1` (Core, exact)

The value is 1 plus the decisions of the function's body:

| Core construct | Adds |
| --- | --- |
| `if a then e else e` | 1 |
| `case a of { ... } [else]` with k branches, `else` included | k − 1 |
| `handle e with x ⇒ e` | 1 |
| a loop (any form of `for`, `spec/17 §4`, `spec/26 §3`, and `together_for`) | 1 |
| `exit_k(...)` (preview) | 1 |
| `first_of(e₁ \| ... \| eₙ)` (preview) | n − 1 |
| `within(...) e` (preview) | 1 |
| `let`, calls, operations, `fail`, `attempt`, `with`, `together`, `tasks`, `start`, `checked`, `declassify` | 0 |

1. The `unreachable` branch of a `when` without `else` (`spec/09 §2`) is not a branch: it does not count in k, and an `if` whose `else` is that branch adds 0.
2. Because of the desugaring of `spec/09 §3`: each `and`, `or` and `implies` adds 1; `x in lo..hi` adds 1; `if none` adds 1; `if error use` and `if error fail` add 1; `if error pass` adds 0; a `when` over k variant arms adds k − 1, and over literal or range arms adds 1 per arm before the last.
3. A lambda's decisions count for its own value and for the enclosing function's. Recursion adds nothing.
4. Refinements, `requires`, `ensures` and predicates of `checked` are not Core expressions and add nothing (§5.3).

Reference values (preset `standard`): `warn` above 10, `block` above 15.

### 5.2. `complexity.cognitive@1` (surface, exact)

Una's definition, after the published principles of Cognitive Complexity: shorthand is ignored, each break in linear flow adds 1, and nesting adds to each break nested in another. It is not equal to any other language's number.

| Surface construct | Adds | Raises nesting for its body |
| --- | --- | --- |
| `if` (first branch) | 1 + nesting | yes |
| `else if` link, `else` | 1 | yes |
| `when` (once, not per arm) | 1 + nesting | yes |
| `for`, `for until`, `for ever` (preview), `together for` | 1 + nesting | yes |
| `if error` clause other than `pass` | 1 + nesting | yes |
| `if none` clause | 1 + nesting | yes |
| `attempt` | 1 + nesting | yes |
| `together` | 1 + nesting | yes |
| `first of` (preview) | 1 + nesting | yes |
| `within` (preview) | 1 + nesting | yes |
| `tasks` (preview) | 1 + nesting | yes |
| `stop`, `skip`, and `fail` not in tail position | 1 | no |
| a run of like boolean operators (`a and b and c`); each change of operator starts a new run | 1 | no |
| a recursive call site | 1 | no |
| a lambda that captures a binding | 0 | yes |

Calls, method chains, `with` clauses, kind declarations and contract clauses add nothing. Nesting starts at 0 in a function's body.

Reference values (`standard`): `warn` above 15, `block` above 25.

### 5.3. Other complexity metrics (exact)

| Id | Repr. | Definition |
| --- | --- | --- |
| `complexity.nesting_max` | surface | The deepest nesting level of §5.2 in the function. `standard`: `warn` above 4, `block` above 6. |
| `complexity.expression_depth` | surface | The depth of the deepest expression tree, before desugaring; an atom has depth 1. |
| `complexity.boolean_terms` | surface | The largest number of condition atoms in one boolean expression. |
| `complexity.loop_nesting` | surface | The deepest nesting of loop constructs. |
| `complexity.exit_points` | surface | `fail`, `stop` and `skip` sites in the function. |
| `complexity.spec` | surface | Atoms plus operators of the function's `requires` and `ensures` clauses and of the refinements of its parameters' and result's types. |
| `complexity.decision_density` | Core | Decisions of §5.1 per 100 `size.function_sloc`; `not_applicable` at 0 lines. |

### 5.4. Halstead (Core, exact)

**Operators** are the Core forms and primitives: `let`, `if`, `case`, `handle`, `fail`, `with`, `attempt`, each `prim`, `call` (every call is this one operator), field access, constructor application, `checked`, `declassify`, and each concurrency form. **Operands** are binders, literals, function and constructor names, and capability operations. Contract clauses are measured apart as `halstead.spec.*`, with the operators and operands of the refinement language.

With n₁ distinct operators, n₂ distinct operands, and N₁, N₂ total occurrences:

| Id | Formula |
| --- | --- |
| `halstead.vocabulary` | n = n₁ + n₂ |
| `halstead.length` | N = N₁ + N₂ |
| `halstead.volume` | V = N × log₂(n) |
| `halstead.difficulty` | D = (n₁ / 2) × (N₂ / n₂) |
| `halstead.effort` | E = D × V |
| `halstead.time` (legacy) | E / 18 |
| `halstead.bugs` (legacy) | V / 3000 |

A value whose formula divides by n₂ = 0 or takes log₂ of n ≤ 1 is `not_applicable`. Every preset sets these to `info`; a project MAY set any of them to `warn` or `block`. The formulas are checked against their primary source before phase 1c implements them; a correction is a new version.

### 5.5. `maintainability.index` (legacy)

MI = 171 − 5.2 ln(V) − 0.23 G − 16.2 ln(L), with V `halstead.volume`, G `complexity.cyclomatic` and L `size.function_total_sloc`, and the scaled form `maintainability.index_scaled` = max(0, MI × 100 / 171). Per function and per module (on the sums), never as a mean alone. `off` in every preset. The coefficients are checked against their primary source before phase 1c; a correction is a new version.

## 6. Duplication

Representation: surface tokens. Cost: O(n log n) time and O(n) space in tokens (a suffix array and its longest-common-prefix array).

1. **Tokens.** The tokens of `spec/01`, without comments and layout. Tokens of `import` lines, `uses` clauses and contract clauses are not code tokens.
2. **Type-1 clones** are maximal repeated runs of at least `min_tokens` tokens spanning at least `min_lines` source lines. **Type-2 clones** are the same after each identifier is replaced by `id` and each literal by its class (`int`, `text`, `float`). Type-3 is `heuristic`, `off` in every preset, and has no definition yet. Type-4 is out of scope.
3. **Metrics.** `duplication.blocks` (clone instances), `duplication.lines` (source lines inside any clone instance, each line once), `duplication.density` = `duplication.lines / size.sloc` per module and package, and `duplication.contracts` (Type-2 clones among contract-clause tokens, with `min_tokens` of its own).
4. **Reference values** (`standard`): `min_tokens` 100, `min_lines` 10, density `warn` above 3%, `block` above 5%. These are starting points to calibrate on Una's corpus.

## 7. Architecture, coupling and dead code

Representation: Core (the call graph and imports). Cost: O(V + E).

| Id | Definition | Unit |
| --- | --- | --- |
| `coupling.afferent` | Ca: modules outside this one that import it or call into it. | module |
| `coupling.efferent` | Ce: modules this one imports or calls into. | module |
| `coupling.instability` | Ce / (Ca + Ce); `not_applicable` when both are 0. | module |
| `coupling.fan_in` | Distinct functions that call this one. | function |
| `coupling.fan_out` | Distinct functions this one calls. | function |
| `coupling.cycles` | Strongly connected components of the module graph with more than one module (Tarjan). | package |
| `coupling.shared_kinds` | Kinds used by more than one module other than their own. | module |
| `coupling.contract` | Other modules whose declarations this module's contracts name. | module |
| `coupling.impact_radius` | The transitive callers of a function, and the declarations whose contracts name it. Only for changed functions and `inspect`. | function |
| `cohesion.kind` | Experimental: the share of a module's functions that take or return a given kind. `heuristic`. | module |
| `api.public_surface_change` | Change of `size.public_items` against the baseline. | module |
| `dead.private_functions` | Private functions with no caller and no reference in the package, examples and tests excluded as callers. | module |
| `dead.private_kinds` | Private kinds that no declaration names. | module |
| `dead.imports` | Imports whose names are not used. | module |
| `dead.public_unused` | Public items with no use inside the package. `info`: a library's callers are outside it. | module |

## 8. Tests, coverage and mutation

Mode: `full`. Coverage comes from the reference interpreter's runs of `una test`, mapped to source spans (§3 rule 3).

1. **Coverage.**
   - `coverage.line`: covered lines over executable lines; a line is executable when a Core node with a span of its own starts on it.
   - `coverage.function`: functions evaluated at least once over functions.
   - `coverage.branch`: decision outcomes taken over decision outcomes, with the decisions of §5.1: an `if`, a `handle`, a `within` and a loop have 2 outcomes each, and a `case` or `first_of` with k branches has k.
   - `coverage.condition`: condition atoms evaluated both true and false over atoms.
   - `coverage.mcdc`: modified condition/decision coverage as LLVM's source-based coverage defines it, with masking for short-circuit evaluation; decisions with one condition are excluded. `off` except in `high_assurance`.
   - `coverage.failure_paths`: declared `fails` variants that some test both produces and handles or passes on, over declared variants.
   - `coverage.boundary`: refinement and contract limits exercised at, below and above the limit, over limits.
   - Native backends: later, through the same span map; disagreement with the interpreter's coverage of the same tests is `inconclusive` with reason `disagreement`. Coverage of Core temporaries, the standard library and the runtime is never reported as the package's.
2. **Testability.** `testability.pure_ratio`: functions with no `uses` over functions. `testability.substitutable_capabilities`: capabilities the package uses for which a handler exists that a test can install with `with`, over capabilities used.
3. **Tests.** `tests.success_density`: (tests − errors − failures) / tests.
4. **Mutation.** A mutant is a copy of the package in memory with one change; the sources on disk are never written. Outcomes: `killed_by_tests`, `rejected_by_verifier`, `survived`, `no_coverage`, `timeout`, `invalid`.
   - `mutation.test_score` = detected / (detected + undetected), with detected = `killed_by_tests` + `timeout`, undetected = `survived` + `no_coverage`. `invalid` mutants are excluded and counted.
   - `mutation.verifier_rejection_rate` = `rejected_by_verifier` / valid mutants. It MUST NOT be added to the test score. The report gives `rejected_by_verifier` as two counts, the mutants rejected by a `refuted` obligation and those rejected only by an `unknown` one, never as one (ADR 0129 rules 7 and 8).
   - `mutation.equivalent_suspects`: survivors the tool flags as possibly equivalent; reviewed by a person, never scored.
   - Reviewed equivalent mutants are entries of the guarantee diff's approvals file at its fixed path (§14.4 rule 4), never of the configuration (ADR 0129 rule 12). Each leaves both denominators and is counted; a new one against the baseline is UQ9302 "a new reviewed equivalence"; UQ9301 keeps its meaning, a new waiver.
   - Every report prints the raw count of each outcome next to each ratio.
5. **Reference values** (`standard`, `warn` only): line 85%, branch 80%, function 95%, test mutation score 80%. Coverage is never shown without the mutation score and failure-path coverage beside it.

## 9. Verification and specification

Mode: `verified`. Read from the facts of `una check`; no proof is redone.

| Id | Definition |
| --- | --- |
| `proof.eligible`, `proof.proven`, `proof.refuted`, `proof.unknown`, `proof.checked_dynamic`, `proof.trusted` | Obligations by status (`spec/12`, `una obligations`). |
| `proof.completion` | `proven / eligible`; `not_applicable` when `eligible` is 0. It covers only obligations that were generated. |
| `proof.cost`, `proof.near_timeout`, `proof.cache_hit_rate` | Solver time and queries per obligation; obligations above a configured share of their limit; cached results over results. `heuristic` (time varies). |
| `contract.coverage` | Public functions with the clauses the configuration requires over public functions. |
| `contract.signals` | Per clause: `ensures` valid with no hypothesis (as `spec/13 §6` `trivial_ensures`), `requires` unsatisfiable, a result no `ensures` constrains, a raw `Int` or `Text` parameter where a refined kind of the package fits. Decided by the solver in the fragment of `spec/12`; otherwise `review_required`. |
| `requirement.traceability` | Requirements with at least one derived function or test over requirements (needs #1289). |
| `escape.checked`, `escape.trusted`, `escape.declassify` | Counts, and densities per 1,000 `size.sloc`, against `[escape_budget]` where it applies. |
| `runtime.residual_checks` | Run-time checks that remain after elision (ADR 0112). Counted, never recommended for removal. |

A public contract weaker than the baseline's (UQ9104) is the guarantee diff's verdict, not a metric of this section: a stronger `requires`, a relaxed `requires` of a public function, a weaker `ensures`, or an incomparable one (ADR 0128 rule 8, ADR 0127 rule 16).

## 10. Authority

Mode: `verified`. An *operation* is a pair `(capability, operation)`.

| Id | Definition |
| --- | --- |
| `authority.declared` | Operations in a function's `uses`. |
| `authority.needed` | Operations reachable from the function through the static call graph, callees' effects and handler bodies included (`spec/05`). |
| `authority.utilization` | `|needed| / |declared|`; `not_applicable` when `declared` is empty. A value below 1 means an operation is declared and unused. |
| `authority.footprint` | The set of operations of a module or package. |
| `authority.breadth` | The sum of the footprint's operation weights (weights per class in the configuration). |
| `authority.propagation` | Per capability, the functions that mention it. |
| `authority.concentration` | The share of the footprint held by the top function or module. |
| `authority.grant_exposure` | Operations granted to `main` that no reachable function uses. |

## 11. Information flow, failure and concurrency

Mode: `verified`. Review signals: a finding here says "review", never "leak". Timing and resource channels are outside the model, and every report lists them (`spec/13 §6` `out_of_model`).

| Id | Definition |
| --- | --- |
| `flow.sensitive_entry_points` | Functions that take or return data labeled above `public`. |
| `flow.declassifications`, `flow.policies` | Counts of policy calls and of policies. |
| `flow.convergence` | Sensitive paths ending at one output function. `heuristic`. |
| `flow.privileged_complexity` | (`complexity.cognitive` / its limit) × Wₐ × W_d, with authority and label weights from the configuration. Orders review; not a probability. `heuristic`. |
| `failure.handling_coverage` | Variants of a function's error kind that some caller handles by value or passes on, over declared variants. |
| `failure.unproduced_variants` | Declared variants that no `fail` produces. |
| `failure.pass_through_chains` | Chains of `if error pass` longer than a limit. |
| `failure.recovery_complexity` | `complexity.cognitive` of `if error` clauses' bodies. |
| `concurrency.task_fanout`, `concurrency.nesting` (preview) | Tasks started per `tasks` block; nesting of concurrency forms. |
| `concurrency.deadline_coverage` (preview) | Operations that can wait and run under `within`, over operations that can wait. Contextual: a pure function needs no deadline. |
| `concurrency.cancellation_coverage` (preview) | Tasks whose cancellation the program handles, over tasks. |
| `loop.termination_evidence` (preview) | Loops with a verified measure over loops; `for ever` counts only under `diverges`. |

## 12. Evolution

1. Every metric has a delta against the baseline: absolute, relative, and status change. A delta between different metric versions is `not_comparable`.
2. Evolution rules: `no_increase` (a ratchet on changed symbols), `no_new` (no new finding at or above a severity), `no_drop` (a coverage or ratio on new code does not fall), and the assurance items of the guarantee diff (#1291, ADR 0128 rule 25): authority expansion, new `trusted`, new declassification, public-contract weakening (a relaxed `requires` of a public function included), and escapes, obligation regressions, toolchain, requirements and assumptions.
3. A policy weaker than the baseline's, rule by rule (a lower severity, a looser limit, a new waiver), is reported (UQ9201, UQ9301). A new reviewed equivalent mutant is UQ9302 (ADR 0129 rule 12).

## 13. Scores

A dimension's score is `Q_d = 100 × (1 − Σ wᵢ pᵢ / Σ wᵢ)`, with versioned public weights wᵢ and penalties pᵢ in [0, 1]. A `not_applicable` value leaves the sums, and the score states its coverage; `not_measured` and `inconclusive` count as pᵢ = 1. There is no score across dimensions. A score never enters a gate or an exit code.

## 14. Configuration

### 14.1. The file

1. The configuration is one file, named by `--config <path>`, by default `quality.toml` at the package root (`quality.una` once ADR 0131's slice lands, #1335). Without either, the preset `standard` applies with no override. One file is read: no include, no inheritance, no environment variables, no conditions.
2. The format is TOML, interim only (ADR 0127 rule 20). The schema below is a data model; its final form is `quality.una`, in Una's own data syntax, which carries the same model and replaces `quality.toml` when its slice lands, converted by `una migrate` (ADR 0131 rules 9 to 13).
3. An unknown key, a value of the wrong type or range, an unknown metric id, preset or code, and a file that is not valid UTF-8 are UQ0001: the run measures nothing and exits with code 1. So are a waiver of UQ9101 to UQ9109 or of a code without severity (§14.4 rule 3), a severity below `block` for UQ9101 to UQ9109 or any severity for a code that has none (§15 rule 5), and an entry of reviewed equivalence (§8 rule 4) (ADR 0127 rules 21 and 28, ADR 0128 rule 24, ADR 0129 rule 12).
4. The file is data. Nothing in it is executed, evaluated or passed to a program.

### 14.2. The data model

```toml
schema = "una.quality-config/1"
preset = "server"                 # a preset name; "standard" by default

[analysis]
mode = "verified"                 # fast | verified | full
incomplete = "block"              # block | report
required = ["not_measured", "partial", "inconclusive", "timeout"]

[scope]
generated = [ { path = "src/gen", reason = "parser tables" } ]

[limits]
mutants = 2000
seconds_per_phase = 600

[metric."complexity.cyclomatic"]
severity = "block"                # off | info | warn | block: the highest severity this rule may give
warn = 10                         # thresholds, in the metric's direction
block = 15
scope = "new"                     # all | changed | new

[metric."halstead.volume"]
severity = "block"                # any metric may block
block = 4000

[evolution."complexity.cognitive"]
severity = "warn"
rule = "no_increase"

[waivers]
max_days = 90

[[waiver]]
rule = "UQ1001"
symbol = "billing.invoice.total"
reason = "table-driven dispatch, to be split"
approver = "billing maintainers"
expires = 2027-01-31
fingerprint = "sha256:..."

[output]
max_recommendations = 5
```

1. Each metric's direction (`above` or `below`) is fixed by its definition. A threshold `info`, `warn` or `block` applies when the value is past it in that direction. A finding's severity is the highest threshold crossed, capped by `severity`. `off` measures nothing for that rule.
2. Every metric of §4 to §11 and every rule of §12 is a valid key, with no exception.
3. Keys absent from the file take the preset's value.
4. **Open in this Draft** (ADR 0131, Decisions of acceptance 4; decided by #1333 before the grammar of `quality.una` lands): how a percentage limit (§14.3 writes `3%` and `85%`) is written, and whether a key carries the metric's version (`complexity.cyclomatic@1`). Proposed default, pending owner confirmation, owned by #1333: a percentage is written as a whole number of points (`85` for 85%), and a configuration key carries the metric id without its `@N` version, since the version is a property of the tool, reported in the output. The example above writes keys that way.

### 14.3. Presets

| Metric | `standard` | `library` | `server` | `high_assurance` | `numerical` |
| --- | --- | --- | --- | --- | --- |
| `complexity.cyclomatic` | warn 10, block 15 | warn 10, block 15 | warn 10, block 15 | warn 8, block 10 | warn 15, block 25 |
| `complexity.cognitive` | warn 15, block 25 | warn 15, block 25 | warn 15, block 25 | warn 10, block 15 | warn 25, block 40 |
| `complexity.nesting_max` | warn 4, block 6 | warn 4, block 6 | warn 4, block 6 | warn 3, block 5 | warn 5, block 8 |
| `complexity.loop_nesting` | info | info | info | warn 3 | info |
| `size.function_sloc` | warn 50, block 100 | warn 50, block 100 | warn 50, block 100 | warn 40, block 80 | warn 80, block 150 |
| `size.sloc` (file) | warn 400, block 800 | warn 400, block 800 | warn 400, block 800 | warn 300, block 600 | warn 600, block 1200 |
| `size.parameters` | warn 6, block 9 | warn 6, block 9 | warn 6, block 9 | warn 5, block 7 | warn 8, block 12 |
| `duplication.density` | warn 3%, block 5% | warn 3%, block 5% | warn 3%, block 5% | warn 2%, block 3% | warn 5%, block 8% |
| `halstead.*` | info | info | info | info | info |
| `maintainability.*` | off | off | off | off | off |
| `coverage.line`, `coverage.branch` | warn 85%, 80% | warn 85%, 80% | warn 85%, 80% | block 90%, 90% | warn 80%, 70% |
| `coverage.mcdc` | off | off | off | warn 100% | off |
| `mutation.test_score` | warn 80% | warn 80% | warn 80% | block 80% | warn 70% |
| `contract.coverage` | warn 80% | block 100% | warn 90% | block 100% | warn 60% |
| `authority.utilization` | warn below 1 | warn below 1 | warn below 1 | block below 1 | warn below 1 |
| Assurance rules (§15) | block | block | block | block | block |
| UQ9101 to UQ9109 (guarantee diff) | block | block | block | block | block |
| UQ9201 policy weaker than baseline | block | block | block | block | block |
| UQ9301 new waiver | warn | warn | warn | block | warn |
| `waivers.max_days` (waivers and approvals) | 90 | 90 | 90 | 30 | 90 |

UQ9101 to UQ9109 take no severity below `block` and UQ9201 has none (§15 rule 5): their rows are fixed, not defaults. The preset's `max_days` also bounds approvals (§14.4 rule 4). A metric the table does not name is `info` with no threshold in every preset. These are reference values to calibrate on Una's corpus, not findings of the literature; a change is a new preset version. No preset switches off a language guarantee. `numerical` does not block dense loops and assumes no proofs about `F64` beyond `spec/29`.

### 14.4. Waivers

1. A waiver has `rule`, `symbol`, `reason`, `approver`, `expires` and `fingerprint`, all required. It suppresses the findings of that rule on that symbol, which the report still lists as waived.
2. It lapses on the day after `expires`, by the run's date (`--date`, else the system date in UTC), or when the symbol's fingerprint (§3 rule 4) differs from the waiver's. `expires` MUST be at most `max_days` after the run's date when the waiver is new against the baseline.
3. A malformed waiver is UQ0001. A lapsed waiver is UQ0003, and the findings it suppressed are reported again. A waiver never approves UQ9101 to UQ9109, and a waiver of one of them is UQ0001; so is a waiver of a code without severity (UQ0001, UQ0002, UQ0003, UQ0102, UQ9201, §15 rule 5) (ADR 0127 rule 28).
4. **Approvals.** The approvals that §15 rule 1 names are entries of the guarantee diff's approvals file, `guarantees.toml` at the package root (interim; `guarantees.una` in its final form, ADR 0131), read from that fixed path only, never from a path a flag or the configuration names, and each bound to one change by its change id (ADR 0128 rules 22 and 23). An approval new against the baseline expires within the preset's `max_days`, and never more than 90 days after the run's date (ADR 0128 rule 22). The same file holds the reviewed equivalent mutants of §8 rule 4 (ADR 0129 rule 12), which approve nothing.

### 14.5. Safety visibility

The report header MUST show the configuration's path and SHA-256 digest, the preset and its version, and every rule set below its preset default (UQ0002). UQ0002 has no severity and cannot be switched off.

## 15. Gates and exit codes

1. **Assurance gate:** UQ0102 (the package does not pass `una check`), UQ0101 (a required analysis is not complete), UQ5001 (a required obligation `unknown` or `refuted`), UQ9101 to UQ9109 (authority expansion, new `trusted`, new declassification, public-contract weakening, escapes, obligation regressions, toolchain, requirements and assumptions, ADR 0128 rule 25), each unless an active approval covers that exact change (§14.4 rule 4), never by a waiver (ADR 0127 rule 28). UQ0102 has no severity: it always blocks. A UQ0101 for a value that comes from the guarantee diff always blocks, under either `incomplete` setting (ADR 0127 rule 12, ADR 0128 rule 24).
2. **Structural gate:** the rules of §4 to §11 other than UQ5001.
3. **Evolution gate:** the rules of §12 other than UQ9101 to UQ9109.
4. **Non-compensation.** The outcome is `pass` only when no gate has a blocking finding. No score, waiver of another rule or other gate's result offsets a blocking finding.
5. **Gate of a code** is fixed by §18 and is not configurable. Its severity is configurable, except:
   - UQ0001, UQ0002, UQ0003, UQ0102 and UQ9201 have none. UQ9201 always blocks, so a configuration injected by path cannot lower the alarm that shows its own lowering (ADR 0127 rule 21);
   - UQ9101 to UQ9109 take no severity below `block`, so `una quality --gate` never passes what `una diff --gate` blocks (ADR 0128 rule 24, ADR 0127 rule 21).

   A configuration that sets a severity for a code of the first kind, or lowers one of the second, is UQ0001.
6. **Exit codes.** Without `--gate`, the run exits 0, or 1 for UQ0001, 2 for a usage error, 3 for an internal error. With `--gate`:

| Code | Meaning |
| --- | --- |
| `0` | No blocking finding; every required analysis complete |
| `1` | At least one blocking finding, or a warning with `--fail-on-warn`, or UQ0001 |
| `2` | Usage error (`spec/13 §5`) |
| `3` | Internal error (U8001) |
| `4` | No blocking finding, but a required analysis incomplete under `incomplete = "report"`, never one that comes from the guarantee diff (rule 1) |

7. Under `--mode fast`, assurance values are `not_measured` with reason `mode`, so the assurance gate cannot pass.

## 16. The finding record

```json
{
  "code": "UQ1001",
  "metric": "complexity.cognitive@1",
  "gate": "structural",
  "severity": "block",
  "status": "complete",
  "label": "exact",
  "symbol": "billing.invoice.total",
  "location": { "file": "src/billing/invoice.una", "start": [12, 1], "end": [58, 12] },
  "value": 27,
  "threshold": { "direction": "above", "warn": 15, "block": 25 },
  "delta": { "baseline": 22, "absolute": 5, "relative": 0.227, "match": "same" },
  "contributors": [ { "start": [20, 5], "end": [31, 9], "construct": "when", "adds": 3, "span": "own" } ],
  "waiver": null,
  "suggestions": [ { "rank": 1, "kind": "extract_function", "lines": [20, 31], "preserves": ["contract", "fails", "uses", "labels", "effects"], "review": false, "confidence": "heuristic" } ],
  "revalidate": ["una check", "una test", "una quality --baseline <snapshot>"]
}
```

Keys appear in this order. `delta` is `null` without a baseline. `match` is `same`, `renamed`, `extracted_from` or `new`. `revalidate` holds fixed command templates, never text from the input. A suggestion never widens authority, adds `trusted` or `checked`, weakens a contract, or edits `quality.toml` or the approvals file.

## 17. Documents

### 17.1. `una.quality/1`

Printed by `una quality --json`, compact, keys in this order:

```text
schema        "una.quality/1"
tool          { version, metrics: { id: version }, presets: { name: version } }
package       { name, version, edition }
config        { path, digest, preset, lowered: [ { rule, preset_default, effective } ] }
date          "YYYY-MM-DD"
mode          "fast" | "verified" | "full"
scope         "all" | "changed" | "new"
baseline      null | { path, digest, tool_version, source_digest }
outcome       "pass" | "block" | "incomplete"
gates         { assurance, structural, evolution }: each "pass" | "block" | "incomplete"
findings      [ finding (§16) ], ordered by gate (assurance, structural, evolution), severity (block first), file, start, code
observations  [ { metric, unit, symbol, value, numerator, denominator, status, reason, label, location } ], in source order
aggregates    [ { metric, scope, max, p95, above: { warn, block }, sum } ]
waivers       [ { rule, symbol, reason, approver, expires, state: "active" | "lapsed" | "invalid" } ]
scores        [ { dimension, value, coverage } ]
out_of_model  [ "timing", "resource consumption", ... ]
```

Adding a key keeps the version; the `metric` ids, `construct` names and `reason` values are open fields that a consumer treats as not understood when unknown; `status`, `label`, `severity`, `gate` and `outcome` are closed.

### 17.2. `una.quality-snapshot/1`

Written by `una quality snapshot --out <file>`: `schema`, `tool`, `package`, `config` (digest and preset), `source_digest` (SHA-256 over the sorted list of each module's path and content digest), `mode`, `date`, and `symbols`: each symbol with its kind, qualified name, fingerprint, and its observations. It holds no value of a run and no finding.

### 17.3. SARIF

`una quality --sarif` prints one SARIF 2.1.0 log with one run: `tool.driver.name` `"una quality"`, its version, and one `rules` entry per `UQ` code used, with its meaning; one `result` per finding, `ruleId` the code, `level` `note` (info), `warning` (warn) or `error` (block), the location as a physical location with the source span, and `properties` holding `metric`, `value`, `threshold`, `status`, `label` and `gate`.

## 18. Code registry

Codes are never reused. Retired codes stay in this table marked retired. Gaps are unassigned numbers (ADR 0012). The thousands digit is the dimension. `una check` never reports a `UQ` code. A row marked *Draft* is assigned by an accepted record and becomes normative with the slice it names.

| Code | Gate | Meaning |
| --- | --- | --- |
| UQ0001 | (none) | Invalid quality configuration |
| UQ0002 | (none) | A rule set below its preset default |
| UQ0003 | (none) | A waiver lapsed |
| UQ0004 | structural | A file under `src/` that is not a module of the package |
| UQ0101 | assurance | A required analysis is not complete |
| UQ0102 | assurance | The package does not pass `una check` |
| UQ1001 | structural | A function's implementation complexity above its limit |
| UQ1101 | structural | A function's specification complexity above its limit |
| UQ1201 | structural | A Halstead measure past its limit |
| UQ2001 | structural | A size past its limit |
| UQ2101 | structural | Duplicated code past its limit |
| UQ2201 | structural | Maintainability index below its limit |
| UQ2301 | structural | Unused code |
| UQ3001 | structural | A coupling measure past its limit |
| UQ3101 | structural | A cycle between modules |
| UQ4001 | structural | Coverage below its limit |
| UQ4101 | structural | Test mutation score below its limit |
| UQ4102 | structural | Verifier rejection rate below its limit (*Draft*: ADR 0129 rule 19, with #1323) |
| UQ4201 | structural | A declared failure that no test produces and handles |
| UQ5001 | assurance | A required obligation is `unknown` or `refuted` |
| UQ5101 | structural | Contract coverage below its limit |
| UQ5201 | structural | A contract that says less than it seems to |
| UQ5301 | structural | Escape density past its limit |
| UQ5401 | structural | Specification sensitivity below its limit (*Draft*: ADR 0129 rule 19, with #1323) |
| UQ6001 | structural | A declared operation that the function never needs |
| UQ6101 | structural | Authority breadth or concentration past its limit |
| UQ6201 | structural | An operation granted to `main` and never used |
| UQ7001 | structural | A flow review signal past its limit |
| UQ7101 | structural | Declassifications past their limit |
| UQ8001 | structural | A failure-discipline measure past its limit |
| UQ8101 | structural | A concurrency-coverage measure below its limit |
| UQ9001 | evolution | A measure worse than the baseline under a ratchet |
| UQ9101 | assurance | Authority wider than the baseline's |
| UQ9102 | assurance | A new `trusted` module or trusted surface |
| UQ9103 | assurance | A new declassification |
| UQ9104 | assurance | A public contract weaker than the baseline's |
| UQ9105 | assurance | A new `checked` site or a raised escape budget (*Draft*: ADR 0128 rule 25, with #1319) |
| UQ9106 | assurance | An obligation regressed from `proven` (*Draft*: ADR 0128 rule 25, with #1319) |
| UQ9107 | assurance | A changed verifier lock or edition (*Draft*: ADR 0128 rule 25, with #1319) |
| UQ9108 | assurance | A requirement removed, changed, activated or left without its derivation (*Draft*: ADR 0128 rule 25, with #1319) |
| UQ9109 | assurance | An assumption stronger than the baseline's (*Draft*: ADR 0128 rule 25, with #1319) |
| UQ9201 | evolution | A quality policy weaker than the baseline's |
| UQ9301 | evolution | A new waiver |
| UQ9302 | evolution | A new reviewed equivalence (*Draft*: ADR 0129 rule 12, with #1323) |

## 19. Explain texts

Drafts of `una explain` for each code. Each final text also holds a minimal example that gives the finding and one that does not, as the `U` codes' texts do, except UQ0001 to UQ0003, whose examples are configuration files.

- **UQ0001.** The quality configuration could not be read: a key is unknown, a value has the wrong type or range, or a metric id, preset or code does not exist. Una Quality measures nothing with an invalid policy, because defaults in its place could pass a gate the project meant to block. Fix the key the message names; `una quality policy check` validates the file alone.
- **UQ0002.** The configuration sets this rule below its preset's default: a lower severity, a looser limit, or `off`. This is allowed. It is always printed, so a change of policy is never silent. If the change was not intended, restore the preset's value or remove the key.
- **UQ0003.** A waiver no longer applies: it expired, or the code it covered changed (its fingerprint differs). The findings it suppressed are reported again. Renew it in a reviewed change, or fix the code.
- **UQ0004.** A file under `src/` is not one of the package's modules (for example a hidden file), so neither `una check` nor Una Quality measures it. Move it out of `src/`, or make it a module.
- **UQ0101.** A value a rule requires was not measured completely: the analysis was partial, timed out, stale, inconclusive or not run in this mode. An unmeasured value never passes. Run in the mode the rule needs (`--mode verified` or `full`), raise the budget, or fix what made the analysis partial.
- **UQ0102.** `una check` rejects the package, so the assurance gate cannot pass. Compiler errors are not quality metrics and cannot be configured. Fix the diagnostics `una check` reports.
- **UQ1001.** A function's implementation complexity (cyclomatic, cognitive, nesting, expression depth, boolean terms, loop nesting or exit points, as the finding's metric says) is past its limit. The contributors list the constructs that add the most. Extract a branch into a function with its own contract, replace nested conditions by a `when`, or move a computation into a refined kind. Keep the public contract, the failure kind, the capabilities and the labels unchanged.
- **UQ1101.** A function's contract is past its specification-complexity limit. Contracts are measured apart from code and are not penalized as code, but a very large contract is hard to review. Name parts of it as predicates, or move conditions into refined kinds.
- **UQ1201.** A Halstead measure of the function is past the limit the project set. Halstead measures correlate with size; reduce the function's vocabulary and length, usually by extracting parts of it.
- **UQ2001.** A file, module or function is longer, or a function has more parameters, than its limit. Contract lines are counted apart and do not add to the function's length. Split the unit, or group parameters into a kind.
- **UQ2101.** A run of code appears more than once (identical, or identical after renaming identifiers and literals). Extract the shared code into a function, or the shared condition into a predicate.
- **UQ2201.** The function's maintainability index is below the limit the project set. The index combines Halstead volume, cyclomatic complexity and length, and is a legacy measure; act on the component the finding names.
- **UQ2301.** A private function, private kind or import is never used. Remove it, or use it.
- **UQ3001.** A module or function depends on, or is depended on by, more others than its limit. Narrow its interface, or move the code that ties it to others.
- **UQ3101.** Modules import or call each other in a cycle. Move the shared declarations into a module both use.
- **UQ4001.** Tests cover fewer lines, branches, conditions or functions than the limit. Coverage is shown with the mutation score, since covered code can still be untested. Add tests that exercise and check the uncovered outcomes the finding lists.
- **UQ4101.** Too many mutants of the code survive the tests: the tests run the code but do not check what changed. Add expectations that tell the mutant from the original. Mutants the verifier rejects are counted apart and never as test kills.
- **UQ4102** (*Draft*). The verifier rejects fewer mutants of the code than the limit the project set. Rejections by a `refuted` obligation and by an `unknown` one are counted apart, and a verifier timeout never counts as a rejection. Strengthen the contracts the surviving mutants pass through.
- **UQ4201.** A variant of the function's error kind is declared but no test produces it and handles it. Add a test that produces the failure and checks its handling, or remove the variant if it cannot happen.
- **UQ5001.** An obligation the project requires is `unknown` or `refuted`. Strengthen a precondition, add a refinement, or fix the code; never remove a check to make the number pass.
- **UQ5101.** Fewer public functions have the contract clauses the project requires than its limit. Add `ensures` clauses that state what the function guarantees.
- **UQ5201.** A contract clause says less than it seems to: an `ensures` that holds with no hypothesis, a `requires` no call can satisfy, a result no `ensures` constrains, or a raw `Int` or `Text` where a refined kind of the package fits. Where the solver cannot decide, the finding is marked for review.
- **UQ5301.** `checked`, `trusted` or declassification is denser than the limit. Each is an escape from proof; prove the property instead where the fragment allows.
- **UQ5401** (*Draft*). Fewer weakened versions of the package's contracts are noticed by independent evidence (a caller, a test, a lock or a requirement) than the limit. A clause nothing outside the function relies on is unobserved; add a test or a caller's obligation that depends on it.
- **UQ6001.** The function declares an operation in `uses` that nothing it calls can perform. Remove the operation from `uses`: least privilege makes the function's authority say what it does.
- **UQ6101.** The authority of a module or package is broader, or more concentrated in one place, than the limit. Split the work so that each function holds only the operations it needs.
- **UQ6201.** A grant to `main` allows an operation that no reachable function uses. Remove it from the grants.
- **UQ7001.** A flow measure (sensitive entry points, convergence of sensitive paths, privileged complexity) is past its limit. This is a signal for review, not a leak: `una check` already rejects every illegal flow. Review the functions the finding lists first.
- **UQ7101.** The package calls policies more often than the limit. Each call lowers a label; review whether each is needed.
- **UQ8001.** A failure-discipline measure is past its limit: variants never produced, long chains of `if error pass`, or complex recovery code. Narrow the error kind, or handle failures closer to where they occur.
- **UQ8101.** Operations that can wait run without a deadline, or tasks without handled cancellation, above the limit. Bound waits with `within` where the program must answer in time.
- **UQ9001.** A measure of a changed function is worse than in the baseline, under a rule that forbids worsening. Bring it back to the baseline's value, or change the rule in a reviewed change.
- **UQ9101.** The package can perform operations the baseline could not. Authority growth needs a recorded approval in the guarantee diff's approvals file, bound to this change; a waiver cannot approve it.
- **UQ9102.** A `trusted` module or trusted surface was added. Trusted code is outside proof; it needs a recorded approval.
- **UQ9103.** A new declassification was added. It lowers a label; it needs a recorded approval.
- **UQ9104.** A public contract is weaker than in the baseline: a stronger `requires`, which breaks callers, a relaxed `requires`, which lets callers through that it kept out, or a weaker `ensures`, which promises less. A simpler implementation never offsets it. Restore the contract, or record an approval.
- **UQ9105** (*Draft*). A `checked` site was added or the escape budget raised. Each escapes proof; it needs a recorded approval.
- **UQ9106** (*Draft*). An obligation that the baseline proved is now `unknown` or `refuted`. Restore the proof, or record an approval.
- **UQ9107** (*Draft*). The verifier's lock or the edition changed, so every proof rests on a different verifier or different rules. It needs a recorded approval.
- **UQ9108** (*Draft*). A requirement was removed, its text or source changed, it became active, or it lost a derived function or its last test. Activation is a person's decision; it needs a recorded approval.
- **UQ9109** (*Draft*). An operation of a capability now promises more about the environment than in the baseline, so every caller assumes more. It needs a recorded approval.
- **UQ9201.** The quality policy is weaker than the baseline's: a rule was lowered, a limit loosened or a rule switched off. It has no severity and always blocks. Review the change of `quality.toml` as you would a change of code.
- **UQ9301.** A waiver was added. Waivers are reviewed exceptions with an expiry; check its reason and approver.
- **UQ9302** (*Draft*). A mutant was recorded as equivalent by review, which removes it from the mutation denominators. Check that the mutant cannot change any behavior the contract admits.

## 20. Limits

Every report states these limits, and this chapter states them for its readers:

1. Classical metrics predict defects weakly and correlate with size; maxima and percentiles are reported, not means.
2. Cognitive complexity has some empirical support for understandability and none for correctness.
3. An agent can move one number by moving another; the paired checks of ADR 0127 section K address this.
4. A passing gate means the configured rules passed. It is not a proof, and properties nobody wrote are not measured.
5. Timing and resource side channels, measured performance and the interaction quality of a product are outside this tool.
