# 0127. Una Quality: a measured, configurable quality report and three gates

- Status: Accepted
- Date: 2026-10-31
- Deciders: Una project (accepted on 2026-11-02, with the changes of "Decisions of acceptance")
- Resolves: issue #1296 (the foundation record of epic #1292)
- Related:
  - issue #1302 and ADR 0131 (Una's own configuration files), whose interim answer this record takes for `quality.toml`, and whose `quality.una` is the configuration's final form;
  - issue #1303 (identifiers), whose scheme this record adopts: codes `UQ` plus four digits, dotted metric ids;
  - issue #1304 (the configuration file, a severity per metric, presets), whose decisions this record states as rules;
  - issues #1297 to #1301 (phases 1 to 5), which implement this record in slices (section L);
  - issue #1291 and ADR 0128 (the guarantee diff), which share baselines and the output model with this record, own the comparison of contracts, and hold the approvals of UQ9101 to UQ9109 (rule 28);
  - issue #1293 (specification mutation testing), which shares the mutation engine of phase 3;
  - ADR 0083 (`una.core/1`, Core IR with source spans), the representation most metrics are computed on;
  - ADR 0012 and `spec/13 §4` (the code registry: codes never reused, gaps unassigned), whose rules the `UQ` registry follows;
  - ADR 0021 and ADR 0122 (project files `una.toml` and `ita.una`), which `quality.toml` stays apart from;
  - ADR 0092 (locked public contracts) and ADR 0093 (authority at module boundaries), whose facts the assurance gate reads;
  - ADR 0109 (profiles `app` and `systems`), whose word "profile" this record does not reuse.
- Target: the toolchain. No construct of the language changes, no diagnostic code `U....` is added, and no edition changes, so no edition-terms question arises (`spec/13 §7`).
- Amends, on acceptance and each with the phase that implements it:
  - **Diagnostics and ABI:** `spec/13 §4` (a sentence that the `UQ` registry lives in `spec/32 §18` and is distinct from the `U` registry), `§5` (the command `una quality`, and `una explain` accepting a `UQ` code);
  - **Specification:** a new chapter, `spec/32-quality.md`, Draft in this change, which holds the metric definitions, the finding record, the documents, the exit codes and the `UQ` registry;
  - **Documents:** `docs/README.md` (the chapter's row).

> **In plain words.**
> - **What it is.** `una quality` measures a package: size, complexity, duplication, coupling, tests, proofs, authority, information flow and failure handling. It prints a report for a person, a JSON document for an agent, and SARIF for CI tools. With `--gate`, it exits non-zero when a rule the project set to `block` is crossed.
> - **What decides.** A file the project owns, `quality.toml` (interim; `quality.una`, in Una's own syntax, once ADR 0131's slice lands), sets for every metric whether it is `off`, `info`, `warn` or `block`, and its limit. Presets give starting values. A pipeline can pass a different file by path.
> - **What it never does.** It never compensates: a good complexity score never offsets a weakened contract. It never reports a value it did not measure as 0 or as passing. It runs no model, edits no file, and treats every input as data.
> - **What it is not.** A proof. A passing gate means the configured checks passed, and the report says what was not measured.

## Questions for acceptance

The decisions below answer the twenty questions of issue #1296 (section B). Three points rest on assumptions that acceptance confirms or changes; the answers are under "Decisions of acceptance":

1. **The safety-visibility rule (rule 21).** Every rule may be lowered by the project, assurance rules included, as issue #1304 decided. This record assumes that the most secure option compatible with that choice is: shipped presets set every assurance rule to `block`; lowering one is always listed in the report header, never silent; and the evolution gate reports a policy weaker than the baseline's (UQ9201, `block` in every preset). The alternative, assurance rules that cannot be lowered at all, is recorded under Alternatives.
2. **Waiver approval (rule 28).** The tool records and displays who approved a waiver, but cannot verify an identity. This record assumes that the repository's own review (the review of the change that adds the waiver) is the approval, and that the tool's job is to make every waiver visible, bounded in time and bound to the code it covers. Signed waivers are deferred.
3. **A new crate (rule 34).** The epic names the crate `una-report` for reuse. That crate is a five-line stub meant for the build report (`spec/13 §6`). This record places the engine in a new library crate, `una-quality`, so that the build report, a tier-1 assurance document, never depends on a policy file the project edits. `una-report` stays as it is.

## Decisions of acceptance (2026-11-02)

The project accepted this record on 2026-11-02 with the changes below. The rules of the Decision carry them; the questions above are kept as they were asked.

1. **Question 1, safety visibility: accepted, with the alarms fixed.** Every rule stays the project's to lower, visibly (#1304, rule 21), except where a configuration swapped by path could hide its own lowering:
   - UQ9201 (a policy weaker than the baseline's) has no severity and always blocks, as UQ0102 (rule 21), and a waiver of it, as of any code without a severity, is UQ0001 (rule 28);
   - UQ9101 to UQ9109, the guarantee diff's codes, take no severity below `block` (rule 21, ADR 0128 rule 24);
   - a UQ0101 for a value that comes from the guarantee diff always blocks, also under `[analysis] incomplete = "report"` (rules 12 and 27).

   The recommended pipeline runs `una diff --gate` (ADR 0128) as the strict gate beside `una quality --gate`.
2. **Question 2, approvals: accepted, and approvals of guarantee changes leave the waiver mechanism.** The review of the change that adds an entry is the approval; signed entries stay deferred. The approvals of UQ9101 to UQ9109 live in the guarantee diff's approvals file, at its fixed path and bound to one change (ADR 0128 rule 22), never in `quality.toml`; a `quality.toml` waiver of one of those codes is UQ0001 (rule 28).
3. **Question 3, the crate: accepted.** The engine is the new crate `una-quality` (rule 34, phase 1a). The question pointed to a rule 46, which does not exist; it now names rule 34, and Question 2 names rule 28 instead of rule 31.
4. **A relaxed precondition of a public function is a weakening** (rule 16), as ADR 0128 rule 8 decides (its Question 1).
5. **The fingerprint covers the body only** (rule 5). The function's header, its contract included, is outside the hash, so a renamed function whose contract changed still matches as `renamed`, and the guarantee diff compares its contract (ADR 0128 rule 7).
6. **The configuration's final form is Una's own syntax** (rule 20). ADR 0131 decides it: `quality.una`, in Una data syntax, replaces `quality.toml` when its slice lands; TOML is interim only. The preset `high-assurance` is spelled `high_assurance`, a word that Una data syntax can carry.
7. **References corrected.** Rule 1's `una explain` item pointed to a rule 40, which does not exist; it names rule 23 and `spec/13 §5`.

Open items, each owned by the card named:
- Whether an active approval of `policy.quality.weakened` (ADR 0128 rule 21) clears UQ9201 for that same change, so that a reviewed, intended lowering of the policy can pass the evolution gate: phase 4, #1300.
- How percentages are written in the configuration, and whether a configuration key carries a metric's version (`@1`): ADR 0131's first slice, #1333. Proposed default, pending owner confirmation, owned by #1333: a percentage is written as a whole number of points (`85` for 85%), and a configuration key carries the metric id without its `@N` version, since the version is a property of the tool, reported in the output.

## Glossary

- **Metric:** a named measurement with a written, versioned algorithm, such as `complexity.cyclomatic@1`. It has a unit of analysis (function, module, package, file) and gives one value per unit.
- **Observation:** one value of one metric for one unit, with its status, label and location.
- **Rule:** a condition on a metric's value, or on its change against a baseline, with a severity chosen by configuration. A rule belongs to one gate.
- **Finding:** an observation that crosses a rule, with a `UQ` code, a severity, a location, the value, the threshold and the contributors.
- **Evidence:** the facts a metric reads beyond the source: obligation statuses, test runs, coverage, mutation outcomes. Evidence has a digest of the inputs it was computed from, and is stale when they changed.
- **Gate:** a set of rules whose blocking findings stop the pipeline. There are three: assurance, structural and evolution.
- **Snapshot:** a document of every observation of a run, with the digests that make it comparable (`una.quality-snapshot/1`).
- **Baseline:** the snapshot a run is compared with.
- **Waiver:** a recorded, expiring exception for one rule on one symbol, with a reason and an approver.
- **Preset:** a named, versioned set of default severities and limits. Not a profile: "profile" names `app` and `systems` (ADR 0109).
- **Mode:** how much evidence a run computes: `fast`, `verified` or `full`.

## Context

Agents write most of the code that Una checks. `una check` decides whether a program is well typed, within its authority, free of illegal flows and proven against its contracts. It says nothing about whether the program is easy to read, change, test or review: a function with forty branches and no tests passes `una check` as well as a short one with full coverage. Agents improve what they can measure, and today nothing measures that.

The epic (#1292) asks for `una quality`: a deterministic report of measured values, readable by a person and by an agent, with an optional gate. The facts it needs already exist:
- Core IR (`spec/09`), in A-normal form, with `and`, `or` and `implies` desugared to `if`, `when` to `case` and `if` chains, and `if error` to `handle`. Its stable export `una.core/1` carries source spans (ADR 0083).
- The surface syntax tree of `una-syntax`, which keeps what Core loses: `when` arms as written, nesting by indentation, comments, and the clauses of a header.
- The build report (`spec/13 §6`): obligations by status, authority per function, policy calls, `checked` counts against `[escape_budget]`, trusted modules, requirements and contract coverage.
- The interpreter's trace (`spec/11 §5`) for coverage, and the obligation listing (`spec/13 §2`).

The forces:
- **Classical metrics predict defects only weakly** and correlate with size. They are still useful as review signals and as a ratchet, if their values are exact, their limits are the project's, and their aggregates are maxima and percentiles, not means.
- **Metrics invite gaming.** An agent can split a function to lower its complexity, add tests without assertions to raise coverage, or move logic into a `trusted` module. Any gate must pair metrics and must never let one dimension pay for another.
- **Una has facts other languages lack:** authority per function, labels, obligations, `trusted` and `checked`. A finding built on them (an unused capability, a weakened contract) is more useful than a classical metric, and some of them are assurance properties, not style.
- **The spec wins and the tool is untrusted input's consumer.** Project files, snapshots and reports are data. The tool runs nothing they name.
- **Configuration is the project's.** Decided for the epic: a separate file, `quality.toml`; a severity for every metric chosen by the user, Halstead included; presets, not profiles; the `UQ` prefix (#1303, #1304).

### Departures from the issues

- **The thresholds of `[quality]` in `una.toml`** (the epic's first sketch) are replaced by `quality.toml`, as #1304 decided.
- **Halstead "never gates"** (the epic's first sketch) is replaced by #1304 rule 4: every metric may be set to `block`. Presets default Halstead to `info`.
- **Duplication on normalized Core IR** (the epic's first sketch) is replaced by surface tokens (rule 9), because desugaring makes different source equal and loses the lines a reader sees.
- **Reuse of `una-report`** is replaced by a new crate (Question 3 above).

## Decision

### A. Shape and vocabulary

1. **The command.** `una quality [path]` measures the package at `path` (`.` by default) and prints the report. Its forms:
   - `una quality [path]`: the human report on stdout; with `--json`, one `una.quality/1` document (`spec/32 §17`); with `--sarif`, one SARIF 2.1.0 log instead. `--json` and `--sarif` together are a usage error, since stdout holds one document (`spec/13 §5`).
   - `--config <path>`: the configuration file; by default `quality.toml` at the package root; when neither exists, the preset `standard` alone.
   - `--mode fast|verified|full` (default from the configuration, else `verified`), `--scope all|changed|new` (default `all`), `--baseline <snapshot>`, `--date <YYYY-MM-DD>` (rule 30), and `--gate` (rule 27).
   - `una quality snapshot [path] --out <file>`: writes the run's `una.quality-snapshot/1`.
   - `una quality policy check [path]`: validates the configuration and prints the effective policy (preset values with the file's overrides, and the list of rules set below their preset default), with no measurement.
   - `una quality inspect <qualified.name>` (phase 5): one function's observations, contributors, preservation constraints and revalidation commands.
   - `una explain <UQ code>`: the explain text of a quality code, through the existing command (rule 23, `spec/13 §5`).
   - Rejected: a `--quality` flag of `una report`. The build report is a tier-1 assurance document whose content depends only on the package; quality findings depend on a policy file. Mixing them would make `una.report/2` vary with a file the project edits.
2. **Relation to `[escape_budget]` and the build report.** `[escape_budget]` stays in `una.toml` and `ita.una`, enforced by `una check` (U7003). `quality.toml` cannot change it. The quality metrics `escape.*` read the same counts and report densities against the budget (rule 15). Quality reads the facts of the build report from the compiler's own data in the same process, never by parsing `una report` output, and adds no key to `una.report/2`.
   - Rejected: moving `[escape_budget]` into `quality.toml`. The budget is a compiler rule with a code; a policy file the pipeline can swap must not loosen it.
3. **Vocabulary.** The terms of the Glossary are normative in `spec/32 §1`. A metric names a measurement; a rule names a condition on it; a finding is a crossed rule; a code names the rule's family.
   - Rejected: one word for metric and rule, as many tools use. A metric with severity `off` still has a definition and may be measured for a score or a delta; conflating the two hides that.

### B. Representation, identity and definitions

4. **Two representations, one per metric (Question 19).** Each metric's definition names the representation it reads:
   - **Core IR** for cyclomatic complexity, decision counts, branch coverage, the call graph, fan-in and fan-out, dependencies and Halstead. Desugaring gives each of them one meaning: `a and b` is one decision whether written inline or in a `when`.
   - **The surface tree** for cognitive complexity, nesting, expression depth, lines and duplication. They describe what a reader sees, which Core loses.
   - **Mapping to source.** Every observation and every contributor has a source span. A Core node without a span of its own takes the span of the innermost enclosing node that has one, and the contributor says so (`"span": "enclosing"`). Phase 1 checks that every decision node of Core maps to a span; a missing span is added to Core as an additive key of `una.core/1` (ADR 0083 rule 3), never guessed.
   - **Consistency.** For each construct that maps one to one, a fixture checks that the decision counts of both representations agree.
   - Rejected: Core IR only. Nesting and lines are properties of the text; Core's A-normal temporaries would add depth no reader sees. Rejected: surface only. Each metric would need its own reading of `and`, `in lo..hi` and `if error`, and the coverage of branches would disagree with the complexity that counts them.
5. **Symbol identity (Question 3).** A function, kind, handler, predicate, policy or capability is identified by its qualified name; a lambda by its enclosing function's name and its ordinal in source order (`m.f#2`). A baseline is matched in three steps, each deterministic:
   1. equal qualified names match;
   2. among the rest, a pair whose **body fingerprint** is equal and unique on both sides matches as `renamed`. The fingerprint is the SHA-256 of the function's body in Core IR, with binder names replaced by their first-occurrence ordinal and spans removed. The header, its contract included (parameters, result, `uses`, `fails`, `requires`, `ensures` and refinements), is outside the hash, so a function renamed with a changed contract still matches, and the guarantee diff compares its contract (ADR 0128 rule 7);
   3. among the rest, a new function whose fingerprint equals that of a contiguous run of `let`s removed from a matched function is reported as `extracted_from` that function, labeled `heuristic`.
   - Unmatched symbols are `new` or `removed`. Ties are never broken by guessing: equal fingerprints shared by several symbols match none of them, and each is `new`. A new symbol falls under the rules for new code, so a rename can only make a rule stricter, never escape it.
   - Cost: O(n log n) in the number of symbols, with sorted maps.
   - Rejected: stable identifiers written in the source. That adds syntax and lets an agent move an identifier to a different function. Rejected: names only. Every rename would reset the baseline's history.
6. **Control flow and complexity (Question 4).** The decision points of each Core construct and the increments of each surface construct are tables of `spec/32 §5`, each metric with a version (`complexity.cyclomatic@1`, `complexity.cognitive@1`). Main points:
   - cyclomatic: 1 plus decisions; an `if` adds 1, a `case` or `first_of` with k branches adds k − 1, a `handle` adds 1, each loop adds 1, each `exit_k` adds 1, each `within` adds 1. The `unreachable` branch of a `when` without `else` is not a branch. `together`, `tasks`, `start`, `attempt`, `fail` and calls add nothing;
   - consequences stated in the spec, not hidden: `x in lo..hi` adds 1 (it desugars to one `and`), and `if error pass` adds nothing (it desugars to its expression);
   - cognitive: Una's own definition, after the published Cognitive Complexity principles (ignore shorthand, add for each break in linear flow, add for nesting), with fixtures for `when`, `if error`, `attempt`, `together`, `first of`, `within`, `for until` and `for ever`. It is not claimed equal to any other language's number;
   - refinements, `requires` and `ensures` are not code: they are measured by `complexity.spec` and `halstead.spec.*`, apart, so a rigorous contract is never counted as complex code.
   - A change of any rule of a table is a new version; reports carry every metric's version, and a delta between versions is `not_comparable` (rule 25).
   - Rejected: building an explicit control-flow graph and computing `E − N + 2P`. For structured Core, the decision count gives the same number in one pass, and the graph adds a structure to keep consistent with Core.
7. **What a line is (Question 5).** `spec/32 §4`:
   - a physical line is a line of the file; a source line (SLOC) has a character that is neither whitespace nor part of a comment; `--` and `---` lines are comment lines; blank lines are neither;
   - the lines of a header's clauses (`requires`, `ensures`, `uses`, `fails`, `derived from`) are `size.contract_sloc`, apart from `size.function_sloc`;
   - examples and tests (`example`, `test`) are their own scope (`tests`); files the configuration lists as generated, with a reason, are the scope `generated`. Una has no marker for generated code, so only the configuration says so. No scope is silently excluded: each is reported on its own, and a file under `src/` that is not a module (a hidden entry, D-009) is reported (UQ0004).
   - Rejected: counting contract lines as code lines. It would punish the clauses Una exists to encourage.
8. **Edition and preview constructs (Question 16).** Una Quality reads packages of every edition. Constructs of edition `0.4`, a preview (ADR 0121), have definitions marked preview: they may change with the edition, by a new metric version. A Core node whose `kind` the tool does not know (an open field, ADR 0083 rule 3) makes the declaration's affected values `partial` with reason `unknown_construct`, never 0. Examples of editions `0.1` and `0.2` and tests of `0.3` and later are the same scope.
   - Rejected: refusing packages of a preview edition. Agents write `0.4` today; a partial, labeled report serves them better than none.

### C. Duplication and coverage

9. **Duplication (Question 6).** Clones are found on surface tokens:
   - Type-1 (equal but for layout and comments) and Type-2 (also equal after replacing identifiers and literals by their classes), as maximal repeated token runs of at least `min_tokens` tokens over at least `min_lines` lines, found with a suffix array and its longest-common-prefix array: O(n log n) time and O(n) space in the number of tokens;
   - `import` lines, `uses` clauses and contract clauses are not code tokens. Duplicated contracts are `duplication.contracts`, reported apart, as a hint toward shared predicates and refined kinds;
   - Type-3 (near-miss) is `heuristic`, off in every preset, and defined later by a new version; Type-4 (semantic) is out of scope.
   - Rejected: fingerprints of normalized Core IR. Desugaring makes distinct source equal, so a finding would point at code that does not look duplicated, and A-normal temporaries break runs. Rejected: rolling hashes alone. Collisions need a quadratic check in the worst case; the suffix array has none.
10. **Coverage (Question 7).** Coverage comes from the reference interpreter first:
    - the interpreter records, per run of `una test`, the Core nodes it evaluated and the outcomes of each decision; the counts are mapped to Una source spans by rule 4;
    - line, function, branch (outcomes over twice the decisions, with the decisions of rule 6), condition, failure path (`fails` variants produced and handled by a test) and boundary (values at refinement limits) coverage; MC/DC as LLVM's source-based coverage defines it, only for presets that ask;
    - native backends later, through the same span map; a run where native and interpreter coverage of the same tests disagree is `inconclusive`, never merged;
    - generated code (Core temporaries, the standard library, the runtime) is never reported as coverage of the package's source.
    - Rejected: instrumenting native code first. Its coverage describes generated code, and mapping it back needs foreign tools Una does not admit.
11. **Interchange (Question 20).** Findings export as SARIF 2.1.0 (`spec/32 §17.3`): the rule id is the `UQ` code, the metric id, value, threshold, status and label go in properties, and locations are Una source spans. Coverage and test-result formats for CI tools (such as LCOV, Cobertura and JUnit XML) are decided in phase 3, after each format is checked against its own primary document; until then only `una.quality/1` carries coverage.
    - Rejected: committing now to a coverage format that has not been read from its source.

### D. Analysis status, evidence and cost

12. **Status on every value (Question 8).** Each observation has a status, `complete`, `partial`, `not_measured`, `not_applicable`, `inconclusive` or `timeout`, and when not `complete`, a reason (`off`, `mode`, `stale`, `budget`, `unknown_construct`, `check_failed`, `edition`, `solver`). A value that was not measured is never 0, never 100%, and never passing. A ratio with an empty denominator is `not_applicable`.
    - **Incomplete analysis.** The configuration's `[analysis] required` lists the statuses a required rule fails on; `[analysis] incomplete` chooses `block` (every preset) or `report`. Under `block`, a required value that is not `complete` is the finding UQ0101 at `block` in the gate of its rule. Under `report`, it is UQ0101 at `warn`, and a run with `--gate` and no blocking finding exits with code 4 (rule 27), so a pipeline can tell "passed" from "not fully measured". A UQ0101 for a value that comes from the guarantee diff (UQ9101 to UQ9109) is at `block` under either setting, as the guarantee diff's own gate always blocks on an incomplete analysis (ADR 0128 rule 24).
    - Rejected: treating an incomplete value as passing, or as failing with no distinction. The first lets a timeout pass a gate; the second hides why the gate failed.
13. **Cost modes (Question 14).**
    - `fast`: the surface tree and Core IR at tier 0; size, complexity, Halstead, duplication, coupling, dead code. O(n log n) in the package's size. For editors and save hooks.
    - `verified`: adds the facts of `una check` at tier 1: obligations, contracts, authority, flow and failure metrics. Proofs come from the proof cache (`spec/12 §9`) and are never redone by the quality tool.
    - `full`: adds test runs, coverage and mutation, within budgets the configuration sets (`[limits]`: mutants, wall time per phase). Reaching a budget gives `timeout`, never a pass.
    - `--gate` needs `verified` at least: under `fast`, every assurance value is `not_measured` with reason `mode`, so the assurance gate cannot pass. The impact radius (transitive callers) is computed only for changed functions and for `inspect`, O(V + E) per function, so no mode is quadratic in the call graph.
    - Rejected: one mode. Editors need answers in milliseconds; mutation takes minutes.
14. **Snapshots, caches and versions (Question 13).** A cached observation is keyed by the tool version, the metric's algorithm version, the configuration digest, the edition, the digest of the inputs it read (source files, and for evidence the lock and the test sources). Any change of a key invalidates it. Evidence whose inputs changed is `not_measured` with reason `stale`, and is shown as stale, never as current. A snapshot made by another tool version is compared metric by metric: a metric whose version differs is `not_comparable`.

### E. Assurance facts

15. **Verification, authority and flow metrics** (`spec/32 §9` to `§11`) are review signals, never vulnerability claims: a finding says "review", never "leak", unless `una check` established a violation, which is then a compiler error and not a metric. Authority utilization (operations declared in `uses` against those reachable through the static call graph) is a new finding: no `U` code reports an unused capability today.
16. **Comparing contracts (Question 12).** Where both contracts are in the decidable fragment of `spec/12`, comparison is by implication in the context of the declaration: a precondition that became stronger (it breaks callers), a precondition of a public function that became weaker (it lets every caller reach what it restricted, such as a removed `requires is_admin(user)`), and a postcondition that became weaker (it promises less) are each a weakening, with the incomparable cases, as ADR 0128 rule 8 states. Each comparison is one solver query, cached as obligations are. Outside the fragment (opaque predicates, a solver `unknown`, a timeout), the result is `review_required`, never "compatible". The comparison is owned by the guarantee diff (#1291); Una Quality reads its verdicts.
17. **Sharing with the guarantee diff (Question 15).** One snapshot family, one output model and one set of baseline rules serve both. The guarantee diff owns contracts, assumptions, obligations, `trusted`, capabilities and declassifications; Una Quality owns structure, tests and coverage. The assurance gate reads the guarantee diff's verdicts for authority expansion, new `trusted`, new declassification and contract weakening (codes UQ9101 to UQ9104), and the further guarantee changes of ADR 0128 rule 25 (UQ9105 to UQ9109). Until #1291 lands, those values are `not_measured`, so the assurance gate under `--gate` with a baseline fails closed. No verification is computed twice.

### F. Gates and scores

18. **Three gates, never compensating (Question 11).**
    - **Assurance:** the package passes `una check` (UQ0102); no required obligation `unknown` or `refuted` (UQ5001); no unapproved authority expansion, new `trusted`, new declassification, public-contract weakening or other guarantee regression against the baseline (UQ9101 to UQ9109, approved only by the approvals file of rule 28); every required analysis complete and current (UQ0101).
    - **Structural:** the thresholds of size, complexity, duplication, coupling, coverage, mutation and the Una-specific metrics.
    - **Evolution:** the comparison with the baseline (ratchets, deltas, new waivers, policy weakening).
    - The outcome is `pass` only when no gate has a blocking finding. The gate of each rule is fixed by the registry (`spec/32 §18`) and is not configurable; its severity is.
    - **Non-compensation.** No score, weight, waiver of another rule or result of another gate offsets a blocking finding. The gate states print before any score.
    - UQ0102 is not a metric and has no severity: a package `una check` rejects never passes the assurance gate (#1304 rule 6).
19. **Scores (auxiliary).** Each dimension may have a score, `Q_d = 100 × (1 − Σ wᵢ pᵢ / Σ wᵢ)`, with public, versioned weights and penalties in [0, 1]. `not_applicable` values leave the denominator, and the score states its coverage; `not_measured` and `inconclusive` count as full penalties. There is no single score across dimensions, and no score is ever a gate.
    - Rejected: a single overall score. One number invites the compensation rule 18 forbids.

### G. Configuration

20. **`quality.toml`** (#1304). A separate file, owned by the project, not a table of `una.toml` or `ita.una`.
    - **By path.** `--config <path>` names it; by default `quality.toml` at the package root. A pipeline injects another file by path (one per environment is an example of use, not a feature). The tool knows no environments.
    - **No merging.** One file is read. It has no include, no inheritance between files, no environment variables and no conditions. The only composition is a preset's values overridden by the file's.
    - **Interim format.** TOML, read by the `toml` crate that `una` already holds for `una.toml` (editions `0.1` to `0.3`), so no dependency is added. The schema (`spec/32 §14`) is a data model independent of syntax. ADR 0131 decides the final form: `quality.una`, in Una's own data syntax, which replaces `quality.toml` when its slice lands, and `una migrate` rewrites the file mechanically (ADR 0131 rule 12). This keeps a TOML reader in `una` past edition `0.4`'s single manifest format (ADR 0122 Question 8.1), accepted for the interim.
    - **Severity for every metric.** Each metric and rule takes `severity`, one of `off`, `info`, `warn`, `block`, and its limits where it has one (`info`, `warn`, `block`, each a threshold, with the metric's direction). This holds for every metric, Halstead, the maintainability index and the legacy measures included: set to `block`, any of them blocks. The finding's severity is the highest whose limit the value crosses, capped by `severity`. `off` measures nothing for that rule and lists it in the header.
    - **Presets.** `standard` (the default), `library`, `server`, `high_assurance` and `numerical`, each versioned (`server@1`), with the values of `spec/32 §14.3`. A preset never switches off a language guarantee, and `numerical` does not block dense loops or assume unrestricted proofs about `F64`.
    - **Validation.** An unknown key, a wrong type or an unknown metric id is UQ0001, which is always blocking and stops the measurement: an invalid policy is never replaced by defaults.
    - Rejected: a `[quality]` table in `una.toml` (the first sketch). The pipeline could not swap it without editing the manifest, and the manifest's format is changing (ADR 0122). Rejected: layered files merged in order. The effective policy would depend on an order the report cannot show in one digest.
21. **Safety visibility for assurance rules** (#1304 rule 7; decided on acceptance, Decisions of acceptance 1). Every preset sets the assurance rules to `block`. The project may lower them, except UQ9101 to UQ9109, which take no severity below `block`: a configuration that sets one lower is UQ0001 (ADR 0128 rule 24). Then:
    - the report header always shows the configuration's path and SHA-256 digest, and lists every rule set below its preset default (UQ0002, always printed, not configurable);
    - the evolution gate reports a policy weaker than the baseline's, by rule (UQ9201). UQ9201 has no severity and always blocks, as UQ0102, so a configuration injected by path cannot lower the alarm that would show its own lowering;
    - the guarantee diff reports the change of `quality.toml` itself (#1291).
    So the choice stays the project's, and a silent weakening by an agent or a pipeline edit is visible in three places.
22. **Who edits the policy.** `una fix` and every repair or recommendation of Una Quality never edit `quality.toml`, never add `trusted` or `checked`, and never widen authority (`spec/13 §3`, §5).

### H. Identifiers

23. **Codes** (#1303). A quality finding has a code `UQ` plus four digits. The thousands digit is the dimension: 0 tool, configuration and analysis status; 1 complexity; 2 size, maintainability and duplication; 3 architecture and coupling; 4 tests, coverage and mutation; 5 verification and specification; 6 authority; 7 information flow; 8 failure, resources and concurrency; 9 evolution and baselines. Codes are never reused, retired codes stay listed as retired, and gaps are unassigned numbers (ADR 0012). The registry is the table of `spec/32 §18`; a test fails on a duplicate or reused code. A code names a rule family; the finding's metric id says which metric crossed it.
    - A `UQ` code is never a compiler diagnostic: `una check` never reports one, and no `UQ` code appears in `una.diagnostics/1`.
    - Rejected: a `U9xxx` range in the compiler's registry. Quality findings are a project's policy, not rejections by the language; one registry for both would let a policy look like a language error.
24. **Metric ids** are lowercase dotted names, `family.measure` (`complexity.cyclomatic`, `authority.utilization`), independent of codes, with an algorithm version (`@1`). They are the keys of `quality.toml`; whether a key carries the version is decided with ADR 0131's first slice (#1333). **Preset names** are lowercase words with a version.

### I. Baselines, evolution and waivers

25. **Baselines (Question 9).** A baseline is a snapshot file. `una quality` reads only files: it never runs `git` or any other program, and never fetches. A pipeline builds the baseline by checking out the base revision and running `una quality snapshot`. A snapshot holds the tool version, metric versions, configuration digest, edition, source digest and every observation with its symbol identity and fingerprint. A delta is absolute, relative and a status change; a delta between different metric versions is `not_comparable`.
    - The baseline is untrusted input: an agent can edit a committed snapshot. The report prints the baseline's digest and claimed source digest, and the recommended pipeline recomputes the baseline from the base revision instead of reading a file from the change under review.
    - Rejected: `--baseline <git ref>` with a built-in reader of repositories. It would run or emulate `git`, whose configuration can run programs, and would put a version-control system inside the trusted base of a quality tool.
26. **Evolution rules.** A rule on a delta takes a severity like any other: `no_increase` on changed functions (a ratchet), `no_new` findings at a severity, `no_drop` of coverage on new code. Scope `new` applies the rules for new code to new or changed symbols (rule 5). The integrity checks of phase 4 pair the deltas (section K).
27. **Gate mode and exit codes.** Without `--gate`, the run is report-only: it exits 0, except for a usage error (2), an invalid configuration (1) or an internal error (3). With `--gate`:
    - `0`: no blocking finding, every required analysis complete;
    - `1`: at least one blocking finding (or a warning, with `--fail-on-warn`);
    - `2`: a usage error;
    - `3`: an internal error (U8001);
    - `4`: no blocking finding, but a required analysis incomplete under `[analysis] incomplete = "report"`, never one that comes from the guarantee diff (rule 12).
    The codes `0` to `3` mean what they mean for every command (`spec/13 §5`); `4` is new and used only by `una quality --gate`.
28. **Waivers (Question 10).** A waiver lives in `quality.toml` as an entry with `rule` (a code), `symbol` (a qualified name), `reason`, `approver`, `expires` (a date) and `fingerprint` (the body fingerprint of rule 5 when it was approved). It suppresses findings of that rule on that symbol only:
    - it lapses when it expires or when the symbol's fingerprint changes, so changed code is measured again;
    - its lifetime is at most `[waivers] max_days` (90 in `standard`, 30 in `high_assurance`);
    - every report lists every waiver, active or lapsed (UQ0003 for a lapsed one; a malformed one is UQ0001);
    - a new waiver against the baseline is UQ9301 (`warn` in `standard`, `block` in `high_assurance`);
    - a waiver never approves UQ9101 to UQ9109, and a waiver of one of those codes is UQ0001, as is a waiver of a code that has no severity (UQ0001, UQ0002, UQ0003, UQ0102, UQ9201), which no configuration can lower. The approval rule 18 names ("unapproved") is an entry of the guarantee diff's approvals file, at its fixed path and bound to exactly one change (ADR 0128 rule 22), so a configuration that a pipeline swaps by path can never carry one.
    The approval itself, of a waiver or of a guarantee change, is the review of the change that adds the entry: the tool records the approver's text and cannot verify it (Questions for acceptance 2).
    - Rejected: approvals recorded by a script or a hook. Arbitrary scripts are code the tool would run (rule 31). Rejected: waivers in a second file. A pipeline would have to inject two files, and their combination would be a merge. Approvals of guarantee changes are not waivers: the approvals file is read from its fixed path only, never injected, so this rejection does not apply to it.

### J. Determinism and agents

29. **Determinism.** The same sources, edition, tool version, configuration, baseline, mode and date give the same bytes on every machine. Iteration follows source order or sorted keys; no model, no randomness. Mutation uses a documented seed, recorded in the report.
30. **The date is an input.** Waiver expiry needs today's date. `--date` gives it; by default the system date in UTC is used, and the report records the date it used, so a rerun with `--date` reproduces it.
31. **Security of the tool (Question 17).**
    - **Inputs are data.** Sources, `quality.toml`, snapshots and waivers are parsed with size and depth limits; unknown keys are rejected; no field is ever executed, evaluated or passed to a shell.
    - **What it runs.** In `fast` and `verified` mode, nothing but the solver children of `una check` (ADR 0124). In `full` mode, tests through the interpreter as `una test` runs them, with its virtual time and without host grants; mutants are made in memory, never written over the sources.
    - **What it reads.** The package's files, the configuration and the baseline named by path. Paths are resolved like a `Files` grant's (`spec/13 §5`): nothing outside the package root through `..` or a link, except the two paths given on the command line.
    - **Output is data.** Names and texts from the source are escaped in the human report (no terminal control sequences) and are JSON strings in documents. Revalidation commands are fixed templates filled with qualified names, never text copied from the input. An agent reading the report treats it as data, never as instructions; the Language Pack (#1295) says so.
    - **No network.**
32. **The agent interface** (phase 5, #1301) is experimental: its value is measured by the benchmark before it is promoted. Recommendations are ranked by blocking first, then impact, confidence and dependencies; each states the preservation constraints (public contract, failure kind, capabilities, labels, observable effects, no new `trusted`) and whether a person must review it. A recommendation that changes a guarantee is never automatic. A refactoring is accepted only after a fresh `una check`, `una test`, quality diff and guarantee diff, and differential tests stay in the list, since contract preservation is not observational equivalence when contracts are incomplete.

### K. Anti-gaming

33. Each way to move a number without improving the code has a paired check:

| Gaming | Check |
| --- | --- |
| Splitting a function to lower its complexity | Paired deltas of fan-out, call depth, public surface and duplicated contracts on the same change (phase 4) |
| Tests without assertions to raise coverage | Coverage is always shown next to the test mutation score and failure-path coverage; never alone |
| Weakening a contract to make code simpler | Assurance gate, UQ9104, from the guarantee diff; a cosmetic gain never offsets it (rule 18) |
| Moving logic into a `trusted` module | Assurance gate, UQ9102; `escape.*` densities |
| Files outside the package's modules | UQ0004: every file under `src/` that is not a module is listed |
| Lowering or switching off rules | UQ0002 in every header; UQ9201 against the baseline; the guarantee diff on `quality.toml` |
| Missing or stale results | Status rule 12: never passing; UQ0101 |
| Renaming to escape the baseline | Unmatched symbols are new and get the rules for new code (rule 5) |
| Editing the baseline | The baseline's digest is printed; pipelines recompute it (rule 25) |
| Long-lived waivers | Expiry, `max_days`, fingerprint binding, UQ9301 (rule 28) |
| Counting the verifier's rejections as test kills | Two separate indicators, never added (`spec/32 §8`) |

The fixture of phase 4: a change that lowers a function's cognitive complexity from 24 to 9 while its public contract loses an authorization condition blocks.

### L. Phases

34. The record is implemented in five phases, each its own card and its own pull requests, each moving its sections of `spec/32` from Draft to Normative as they land with their tests:
    - **Phase 1, #1297, structural engine.** (1a) The crate `una-quality`, the configuration reader and `una quality policy check`, the `UQ` registry and `una explain UQ....`, the `una.quality/1` skeleton with statuses; (1b) size and complexity, with a fixture of known values for every construct of every edition; (1c) Halstead and the maintainability index; (1d) coupling, cycles and dead code; (1e) duplication; (1f) snapshots and SARIF. Report-only: no `--gate`.
    - **Phase 2, #1298, assurance metrics.** Verification, contract signals, authority, flow, failure and concurrency metrics, from the facts of `una check` in `verified` mode; false-positive rate measured on the examples.
    - **Phase 3, #1299, tests.** Interpreter coverage, the mutation engine shared with #1293, boundary coverage, the CI formats of rule 11. Runs of the mutation corpus are scheduled with the other heavy evidence before a release.
    - **Phase 4, #1300, gates.** `--gate`, exit codes, presets, baselines and evolution rules, waivers, integrity checks, seeded regressions for every rule. Needs #1291 for UQ9101 to UQ9104.
    - **Phase 5, #1301, agent interface.** `inspect`, recommendations, the documented loop, its entry in the Language Pack.
    - The amendments of `spec/13 §4` and §5 land with phase 1a; each later phase amends only `spec/32`.

### M. Out of scope

35. A metric that needs a model; any rewrite of code by the tool; style rules the formatter fixes (`spec/15`); measured performance (the benchmark repository measures it); interaction capability (diagnostics and documentation are measured by the benchmark and the Language Pack); delivery metrics of teams; timing and resource side channels, which every report lists as outside the model.

## Complexity

Every metric's cost is in `spec/32`. In summary, for a package of n tokens, V functions and E call edges:
- size, complexity, Halstead and dead code: O(n), one pass over each representation;
- duplication: O(n log n) time, O(n) space (suffix array);
- coupling and cycles: O(V + E) (Tarjan's strongly connected components);
- symbol matching against a baseline: O(V log V);
- impact radius: O(V + E) per function asked, only for changed functions and `inspect`;
- verification facts: no new solver query except contract signals and contract comparisons, one per clause, cached;
- mutation: one test run per mutant, bounded by the configured budget.

Scaling tests at growing sizes, failing on quadratic behavior, come with phase 1 for every pass above.

## Security

Rule 31 states the tool's own security. The assurance gate cannot be passed by an incomplete analysis (rule 12), by a lowered rule without a visible trace (rule 21), or by a score (rule 18). The tool adds no code to the trusted base of a checked program: it never changes what `una check` accepts, and a quality gate is an additional condition, never a replacement for a check.

## Verification

No obligation kind is added. Contract comparison and contract signals (rules 16, 15) are solver queries in the fragment of `spec/12`, cached like obligations, whose `unknown` is `review_required`.

## Information flow

Una Quality reads labels and policy calls as type information and reports counts and locations, never values. A report holds no value of a run: coverage counts evaluations, and a mutation outcome is a status. Findings about flow are review signals (rule 15).

## Diagnostics

No `U` code is added. The `UQ` registry starts with the codes of `spec/32 §18`, one per rule family, each with an explain text. `una explain` accepts both registries; consumers that assumed `U[0-9]{4}` (the Agent ABI schema, the language server, `una explain`) are checked in phase 1a and amended by dated sections where needed.

## Consequences

- Agents and people get exact, versioned measurements with locations and contributors, and a gate whose policy the project owns.
- Una's own facts (authority, labels, obligations, escapes) become review signals that classical tools cannot give.
- A project can block on any metric, Halstead included, and the report always shows what it lowered.
- The quality tool keeps a TOML reader until ADR 0131's slice for `quality.una` lands.
- Phase 4's assurance items depend on the guarantee diff (#1291); until it lands they fail closed under `--gate` with a baseline.
- Reference limits are starting points to calibrate on Una's corpus, not findings of the literature; calibration may change preset values, which is a new preset version.
- Deferred: Type-3 clones, CI coverage formats, native coverage, signed waivers, abstractness over capabilities and handlers, a static cost metric (`costs` is undecided).

## Alternatives considered

- **No tool; leave quality to review.** Rejected: agents improve what they can measure, and review does not scale with generated code.
- **Wrap an existing analyzer.** Rejected: no existing tool reads Una, and a foreign analyzer would be foreign code in the toolchain.
- **Thresholds hard-wired as non-blocking for weak metrics.** Rejected by #1304: which metrics block is the project's policy.
- **Assurance rules that can never be lowered.** The most restrictive option. Rejected on acceptance in favor of rule 21, because #1304 made every rule the project's choice, with UQ9201 and UQ9101 to UQ9109 fixed (Decisions of acceptance 1).
- **A single overall score as the gate.** Rejected: rule 18.
- **Model-based findings (summaries, "smell" detection).** Rejected: non-deterministic, and a gate must give the same answer on every machine.

## Implementation plan

Section L. Each phase is its own card; each pull request lands the spec sections it implements with fixtures of known values, and a test that the registry has no duplicate or reused code.

## Conformance

- A fixture per metric and per construct, with its value computed by hand in the fixture's comment.
- Determinism: two runs, byte-identical documents.
- Each severity changes the exit code as rule 27 states; Halstead set to `block` blocks; an unknown key is UQ0001; a file that lowers an assurance rule prints UQ0002.
- A rename keeps a function's history; an ambiguous fingerprint matches nothing.
- The anti-gaming fixture of section K blocks.
- No value with a status other than `complete` is ever counted as passing.
