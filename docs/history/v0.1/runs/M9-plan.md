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

Every rule of the milestone's sections. `spec/13 §4` to `§6` and `spec/08 §1` state no MUST, MUST NOT or MAY that M9 implements (the MAY of `spec/13 §3` is M8's), so the table lists their rules. Evidence names tests of `crates/una-cli/tests/abi.rs` (A), `crates/una-cli/tests/cli.rs` (L), the explain test `crates/una-cli/tests/explain_examples.rs` (E) and unit tests of `crates/una-verify` (U).

### Rules of the sections

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/13 §1`: compact JSON with object keys in the order shown; deterministic output | done: every document is a struct serialized in the order of its section, `schema` first | A `every_command_prints_one_json_document_with_its_schema`, `json_outputs_are_byte_identical_across_runs_and_thread_counts` |
| `spec/13 §1`: exit codes do not depend on `--json` | done | A `report_and_trace_exit_with_1_on_an_unproven_obligation` |
| `spec/13 §2`: `una obligations --json`, hypotheses as in the example | done: hypotheses read as source, temporaries through their definitions or source text, with the precedence of `spec/01 §7` | A `the_obligations_of_the_events_example` (snapshot); U `formulas_show_as_source_with_the_parentheses_of_the_precedence_table` |
| `spec/13 §4`: `una explain <code>` prints a paragraph and a minimal failing and fixed example for every code | done for every code but U8001, whose text has no example (D-126) | E `failing_examples_yield_their_code_and_fixed_examples_yield_nothing` |
| `spec/13 §5`: `una trace` and `una report` | done (D-122, D-124) | A |
| `spec/13 §5`: `--json` for every command, nothing else on stdout, stderr empty but for U8001 | done, with `una.explain/1` (D-125) | A `every_command_prints_one_json_document_with_its_schema`; L `explain_json_prints_its_document` |
| `spec/13 §5`: exit codes 0, 1, 2 and 3 | done: 1 with an unproven obligation or a rejected package, 2 for an unknown requirement, 3 for U8001 | A `report_and_trace_exit_with_1_on_an_unproven_obligation`, `trace_lists_the_functions_derived_from_a_requirement` |
| `spec/13 §6`: `una.report/1` with its sections in order | done (D-122) | A `the_report_of_the_events_example` (snapshot), `a_single_file_is_named_after_its_module` |
| `spec/13 §6`: `trivial_ensures` counts clauses the solver proves without any hypothesis, reported as missing contracts | done: one query per clause of a public function (D-123) | A `trivial_ensures_count_as_missing_contracts` |
| `spec/13 §7`: `[package] name` and `version` | done: the manifest keeps them for the report | A `trivial_ensures_count_as_missing_contracts` |
| `spec/07 §5` rule 5: every policy call site in the Information section, with the policy name, purpose and owner | done | A `the_report_of_the_events_example` (the call at 30:13) |
| `spec/07 §6`: trusted modules and their line counts in the TCB section | done | A `the_report_of_the_events_example` (`events.keys`, 17 lines) |
| `spec/06 §6`: the escape budget | done: `checked` per module against `checked_per_module` | A `the_report_of_the_events_example` |
| `spec/08 §1`: `una trace` lists every function deriving from the requirement, its contract clauses and the status of each obligation | done (D-124) | A `trace_lists_the_functions_derived_from_a_requirement` (snapshot) |
| `compiler/testing.md §4`: identical output twice and with one thread | done for every document | A `json_outputs_are_byte_identical_across_runs_and_thread_counts`; L the determinism test of `una check --json` |

### Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| The conformance runner compiles every `explain` example, which fails with its code, and its fixed version passes | E, over every registered code but U8001 (D-126); conformance runner: 190 cases, 190 passed, 0 pending |
| Report and obligations JSON snapshots for the events example | A `abi__report_events_json.snap` and `abi__obligations_events_json.snap`, reviewed |

### Found during the milestone

- A `let` bound to `and`, `or`, `implies`, `in` or `is` held only the disjunction of its paths, since Core lowers them to branches; `spec/03 §7` selfifies such bindings, and they now hold `x = ite(...)`. The queries of the reference example got simpler, and no status changed.
- The listing showed temporaries (`%t0.remaining`, `valid_phone(%t2)`) and dropped the parentheses under `not`; hypotheses now print through definitions and source text, with the precedence of `spec/01 §7`.
- The report ignored a solver failure on the trivial `ensures` queries, which would have understated the missing contracts; it reports U5003 or U8001 now, as tier 1 does.
