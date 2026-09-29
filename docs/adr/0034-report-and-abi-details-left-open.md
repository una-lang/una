# 0034. The build report and the ABI details the specification leaves open

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo (accepted on 2026-09-29), on a proposal by Claude (overnight run)

## Context

M9 completes the Agent ABI of `spec/13`: the build report of `§6`, `una trace` of `spec/08 §1`, `--json` for every command, and explain texts for every code. `spec/13 §6` gives one example of `una.report/1` without saying which functions, calls, escapes and capabilities each section lists, how the lock is hashed or what a trivial `ensures` query is; `spec/08 §1` names `una trace` without an output; `spec/13 §5` makes `--json` a global flag, while `una explain` has had no JSON output since M1; and `spec/13 §4` asks a failing example of every code, which an internal error cannot have.

Decision log entries: D-122 to D-126 (`docs/reviews/open-decisions.md`).

## Decision

1. **The report's contents.** `una report [path]` checks the package at tier 1; when tier 0 reports diagnostics it prints them as `una check` does, and otherwise the report, with exit code 1 when an obligation is not proven. The sections, in the key order of `spec/13 §6`:
   - `package`: the name and version of `una.toml`; a single file is named after its module, with version `0.0.0`.
   - `lock`: the solver version and the hex SHA-256 of the lock's contents (of the default lock's text for a single file).
   - `proven`: the number of proven obligations as `semantic`; `security` and `authority` are `"by typing"`. `obligations`: the counts by status.
   - `authority`: every function of the package with `uses`, by qualified name, with `public`, its `uses` as written in a header, and whether they are inferred.
   - `information.policy_calls`: every call of a policy, in source order, with the policy, its purpose and owner, and the site.
   - `escapes.checked`: every module that uses `checked`, with the count and the budget of `una.toml`.
   - `assumptions`: every capability the package declares, with its operations and their `ensures` clauses as `op: clause`.
   - `tcb`: the trusted modules with their line counts, the host capabilities that the package's functions use, and the solver as `z3 <version>`.
   - `requirements`: every requirement, by identifier, with its status and the functions derived from it.
   - `coverage`: rule 2. `out_of_model`: the four channels of the example.

   Without `--json`, the same sections print as labeled lines.
2. **Trivial `ensures` and coverage.** An `ensures` clause is trivial when the solver proves it with no hypothesis, its parameters and `result` unconstrained: one more query per clause of a public function, cached like the others. A solver that cannot answer these queries gives U5003 or U8001, as in tier 1, instead of the report. `pub_functions` counts the public functions of the package, `with_ensures` those with at least one `ensures` clause that is not trivial, `with_derived_from` those with a `derived from` clause, and `trivial_ensures` the trivial clauses of public functions.
3. **`una trace <REQUIREMENT_ID>`** checks the package in the current directory at tier 1, as `una contract` does, and prints the requirement (identifier, status, text, source, owner), then every function derived from it, by qualified name, with its `requires` and `ensures` clauses and the status of each of its obligations. `--json` prints `{"schema":"una.trace/1","requirement":{...},"functions":[{"function":...,"requires":[...],"ensures":[...],"obligations":[{"id":...,"kind":...,"status":...,"goal":...}]}]}`. An unknown identifier is a usage error (exit code 2); unproven obligations give exit code 1.
4. **`--json` for every command.** Every command prints one JSON document with a `schema` field. `una explain --json <code>` prints `{"schema":"una.explain/1","code":...,"domain":...,"meaning":...,"text":...}`. When the checks reject the package, `una contract`, `una trace`, `una obligations` and `una report` print the diagnostic list of `una check` (ADR 0012), and `una run` and `una test` print their own documents, which carry the diagnostics. Usage errors stay on stderr (D-010).
5. **U8001's explain text** has no failing or fixed example: an internal error is a bug of the compiler, and the text says how to report it. Every other code has both examples.

### Changes to other documents on acceptance (made on 2026-09-29)

- `spec/13 §6` states rules 1 and 2; `spec/08 §1` rule 3; `spec/13 §5` rule 4, with the two schemas; `spec/13 §4` rule 5.

## Consequences

- Agents read every command through `--json`, with one document per run and a schema to dispatch on.
- The report grows by one query per `ensures` clause of a public function, which the cache absorbs.

## Alternatives considered

- **Trivial `ensures` without the solver, by syntax:** catches `ensures true` but not `ensures result == result` or `ensures x >= x`.
- **A JSON error document for usage errors:** no code exists for them, and D-010 keeps them on stderr.
- **Examples for U8001 through a deliberately wrong input:** there is none; a known input that crashes the compiler would be a bug to fix.
