# The differential security hunt

- Issue: #513 (part (c): #514)
- Date: 2026-10-10
- Status: approved (2026-10-10), with the conditions of §6

## 1. Why

The formal track found S-14 to S-18 by comparing the checker with the Lean model (F1) and its re-check. This plan makes that comparison systematic, and adds an oracle that does not need the model.

The re-check sees little of the language today. On the corpus of main 56ce83ed (examples and `tests/conformance`, 1,344 targets exported by that main's own `una --json core`), it accepts 760 of 4,238 declarations (18%) and rejects none:

- 2,451 are in incomplete documents. When `una check` reports a diagnostic, the export writes `complete: false` and leaves out the declarations with diagnostics (`spec/13 §8`), and the re-check does not read the rest.
- Most of the others use a construct F1 lacks:
  - `pred` (111), `handler` (84), `via` (57, by design: no label is checked in a `via` function);
  - the primitives `div`, `mul`, `neg` and `le` (81);
  - `fixed` (37), `case` over a data kind (37), user records (about 200; #509);
  - `Map` (17), `with` (16), `together` and `together_for` (26), the `Ord` constraint (11);
  - prelude `first`, `or_else`, `min` and `is_ok`, `Json`, and failures with a payload.
- The label lattice is not a gap: F1 has the five levels, categories, and the symbolic labels `ℓ_T` and `ℓ*_T`.

So a checker–model comparison finds bugs only inside F1's fragment. S-14 to S-18 were there, or at its edge.

## 2. The pipeline

A generator or mutator writes a program. Then:

1. `una check` decides whether it is accepted.
2. For an accepted program, `una --json core` exports it, and the re-check (`una-core-check`) reads it: part (b).
3. The two-run oracle runs it: part (e).
4. A candidate is minimized and triaged (§4).

## 3. The parts

### (a) Programs

- **Mutators** over the `ok` and `run` conformance programs and the examples. Each operator is tagged with the spec/07 rule it attacks:
  - raise a literal through an annotated `let s: secret T`;
  - put a branch on a secret value around a sink, a call, a `fail` or a `Files.write`;
  - wrap a call in `attempt`, `handle` or `if error pass`, `use` or `fail`;
  - route a value through a generic, a lambda, `fold`, `map` or `first_where`, a record field, a kind parameter (variance), `Option`, `Result` or a tuple;
  - change a policy's `to`, or make its `via` function fail or use `checked`;
  - move code into a handler body, a `together` block or an `of` parameter;
  - change the edition.
- **Targeted generators**, depth-bounded, one per rule cluster:
  - pc and failures (`spec/07 §3` rules 2 and 3);
  - flows and variance (§7 rule 2);
  - operations, the effect floor and the file-state label (§7 rule 4);
  - policies (§5, §7 rule 5);
  - generics: `ℓ_T` and `ℓ*_T`, result summaries, edition 0.1's defense in depth (§7 rule 8);
  - `of` clauses and the prelude's higher-order functions (§7 rule 9).
- Generators are Python, with no cargo, and record a seed for every program.
- The operators, their expected verdict classes, and two cheap oracles that need only `una check` (a mutant that injects a forbidden flow must be rejected; a refactor that keeps every label must keep the verdict) are in `docs/plan/security-hunt-generators.md`.

### (b) Checker against model

A declaration that `una check` accepts and the re-check rejects is a candidate finding. The re-check rests on T6, so a rejection names either a leak the checker misses, or a bug of the model or its importer.

### (c) The reverse (#514)

It needs an additive export mode that also writes the rejected declarations, with their diagnostics, in their own section or under a marker, never as accepted Core (ADR 0076). Then:

- A declaration the checker rejects with U400x and the model accepts is either an over-strict checker, which is a completeness matter, or a channel that the checker guards and the model lacks. The second is a blind spot of T6, and the security-relevant outcome.
- A declaration the checker accepts and the re-check does not re-check is a coverage gap, counted per construct for the formal track.

### (d) The audit of spec/07

A table of every rule and sub-rule of `spec/07` §1 to §7, with:

- the checker's site;
- the conformance tests;
- the model's rule or theorem;
- the importer's support and the corpus count;
- a status: covered, partial or blind.

§1's channels get rows of their own: return values, failures and their payloads, operation arguments, the occurrence of effects. The out-of-model list of §1 is checked against what the model actually omits. The table lives in `docs/reviews/security-hunt/coverage.md`.

### (e) The two-run oracle

For each accepted program, two variants differ only in a literal behind `let s: secret T = <literal>`. That flow from `public` into `secret` is legal, so both variants are accepted. Both run on both hosts: the interpreter always, native on a sample. A public observer's view is compared:

- standard output;
- the exit code and which failure;
- the effects on `public` files and on the mock Http server.

Any difference outside §1's exemptions is a concrete leak witness, with no model involved. The exemptions are termination, timing, resources, and the aborts U6003 to U6005.

- For state that outlives a run, as S-15's file state does, each variant runs twice on the same file system.
- The oracle finds only what its inputs exercise, so the generators also emit boundary literals: zero, negatives, empty and long text, and invalid UTF-8 for host inputs.
- It sees every construct, including those F1 lacks.
- **Targets, in order.** First, the rules no theorem covers (`docs/reviews/security-hunt/coverage.md` §2). Among them, trusted modules and policy shapes come first, since the model assumes every policy is lawful (decided 2026-10-10).

## 4. Triage

- Candidates are deduplicated by rule, construct and outcome, and each is kept with its seed and its minimized source.
- A confirmed leak follows the S-xx process. It is reported privately first, with the minimized repro, before any fix. Then come a repro in `tests/conformance/security`, an ADR when a rule changes, a fix in both editions, and patch releases.
- A bug of the model or the importer is an issue of the formal track.
- A channel outside the model gets a note in `spec/07 §1`, or an ADR if it should enter the model.

## 5. Cost and machine

Light work, possible while the benchmark runs (reading, Python and `gh` only):

| Step | What | Estimate |
|---|---|---|
| L1 | (d), the audit table | 1 session-day |
| L2 | The mutation operators and the generators' design | 0.5 day |
| L3 | The oracle's spec: observations, exemptions, sandbox, mocks | 0.5 day |
| L4 | The design of (c)'s export mode | 0.25 day |

Heavy work, only in machine slots between benchmark runs:

| Step | What | Estimate |
|---|---|---|
| H1 | Running the generators against the compiler while developing them | a few hundred runs |
| H2 | (c)'s export mode: cargo and local-ci | 0.5 day |
| H3 | Campaigns: per round, about 10,000 programs, each checked, exported, re-checked and run twice on the interpreter; native on about 5% | 1 to 3 machine-hours per round, measured on the first 500 |
| H4 | Triage and minimization | 0.5 to 2 hours per candidate |

## 6. Conditions (2026-10-10)

1. The oracle runs only in a sandbox: temporary directories, no network but the mock Http server, and `Files` grants scoped to a fresh directory per run.
2. Every confirmed leak is reported privately first, with its minimized repro, before any fix.
3. Campaigns, and any run of the compiler, take machine slots between benchmark runs, never during one, even at one job.
4. (c)'s export mode is additive (ADR 0076), and rejected declarations are never presented as accepted Core.
