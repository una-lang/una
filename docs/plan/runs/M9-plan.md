# M9 plan: Agent ABI and build report

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/13` (the Agent ABI: diagnostics, the obligation listing, repairs, the code registry, commands, the build report, project files), `spec/08 §1` (requirements and `una trace`); also `spec/07 §5` rule 5 and `§6` (policy call sites and trusted modules in the report), `spec/06 §6` (the escape budget), `spec/12 §6, §10` (statuses and determinism), `compiler/testing.md §4` (determinism tests), ADR 0012 (the diagnostic list), ADR 0015 (output streams) and ADR 0029 (`una contract`).

Codes: none new. Acceptance (`plan/milestones-v0.1.md`): the conformance runner compiles every `explain` example and checks that it fails with its code and that the fixed version passes; report and obligations JSON snapshots for the events example.

## 1. Design

- **`una report [path]`** checks the package at tier 1 and prints the build report of `spec/13 §6`: the package, the lock with its hash, the proof counts, the authority of every function with `uses`, every policy call with its purpose and owner, the `checked` escapes per module against the budget, the contracts of the package's capabilities as assumptions, the trusted computing base (trusted modules with their line counts, the host capabilities the package uses, the solver), the requirements with the functions derived from them, the coverage of public functions (with `ensures`, with `derived from`, trivial `ensures`), and what is out of the model. `--json` prints `una.report/1`.
- **Trivial `ensures`** (`spec/13 §6`): an `ensures` clause the solver proves with no hypothesis at all, whatever the parameters and the result, such as `ensures true` or `ensures result == result`. Each clause of a public function is one more query, cached like the others; a trivial clause counts as a missing contract.
- **`una trace <REQUIREMENT_ID>`** (`spec/08 §1`) checks the package in the current directory at tier 1, as `una contract` does, and lists the requirement, every function derived from it with its `requires` and `ensures`, and the status of each of the function's obligations; `--json` prints `una.trace/1`.
- **`--json` everywhere** (`spec/13 §5`): every command prints one JSON document with a `schema` field. `una explain --json` prints `una.explain/1`, with the code, its domain, its meaning and its text, instead of the usage error of M1. A command whose document describes a checked program (`contract`, `trace`, `obligations`, `report`) prints the diagnostic list of `una check` when the checks reject the package; `run` and `test` carry the diagnostics in their own documents. Exit codes follow `spec/13 §5` for every command.
- **Explain texts**: U8001 gets its full text. It has no failing example, since an internal error is always a bug of the compiler; every other code keeps a failing and a fixed example, which the explain test compiles.
- **Determinism** (`compiler/testing.md §4`): the determinism test covers the JSON of `una report`, `una obligations` (with a warm cache), `una test`, `una contract` and `una trace` on the examples, twice and with one thread.

## 2. Spec gaps and ADR 0034

ADR 0034 (Provisional), "the build report and the ABI details the specification leaves open", records:
1. **The report's contents** (D-122): which functions, policy calls, escapes, capabilities and host capabilities each section lists, how the lock is hashed, how a single file names its package, and the human form.
2. **Trivial `ensures` and coverage** (D-123): the query without hypotheses, and which functions each coverage count includes.
3. **`una trace`** (D-124): the package it reads, its human form, `una.trace/1`, and its exit codes.
4. **`--json` for every command** (D-125): `una.explain/1`, and which document a rejected package gets.
5. **U8001's explain text** (D-126): a text without examples.

## 3. Tasks and commits

1. **Plan and ADR 0034** (`docs(plan)`).
2. **`una report`** (`feat(cli)`, `feat(verify)`): the sections, trivial `ensures`, the human form and `una.report/1`, snapshots of the events example.
3. **`una trace`** (`feat(cli)`): human form and `una.trace/1`.
4. **`--json` everywhere and explain** (`feat(cli)`, `docs(diag)`): `una.explain/1`, U8001's text, the explain test over every code.
5. **Determinism** (`test(cli)`): the JSON of every command on the examples.
6. **Self-review** (`docs(plan)`).

Each commit is pushed once it passes.

## 4. Conformance cases

The report and the trace are package outputs, tested by CLI snapshots rather than directives; the conformance runner and the explain test already compile every example of `spec/13 §4`.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Every `explain` example fails with its code, and its fixed version passes | `crates/una-cli/tests/explain_examples.rs`, over every code but U8001 |
| Report and obligations JSON snapshots for the events example | insta snapshots of `una report --json` and `una obligations --json` |

## 6. Known gaps

- The semantic diff that would surface a requirement changing from `generated` to `human` is v0.3 (`spec/08 §1` rule 2).

## 7. Self-review checklist

Appended when the milestone is complete.
