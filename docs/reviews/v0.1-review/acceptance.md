# Acceptance audit, M0 to M10

Tree: `main` at `c2f3386`. Z3 5.1.0. Every result below was reproduced by the reviewers, and commands that write files were run against a copy of the repository.

**Summary: 28 criteria; 27 pass, 1 partial, 0 fail.**
- The partial one is M10's "release artifacts build in CI": only the Linux archive has been built there (R-035).
- The acceptance criteria, as written, do not catch the Critical soundness findings (R-001 to R-011). Every criterion of M5, M6 and M8 passes, but the guarantees those milestones deliver (authority, information flow and verification) are broken by programs that no conformance case exercises.
- The recommendation is to treat M5, M6 and M8 as not done until those findings are fixed and each has a conformance case.

## M0. Workspace and infrastructure

| Criterion | Status | Evidence |
|---|---|---|
| `cargo test --workspace` passes | Pass | Lead baseline (see `summary.md`): `== test exit 0`, also with threads=1; zero `N failed` lines with N > 0. Targeted runs of every una-cli test target, una-diag, una-syntax, una-fmt tests all `ok` (below). |
| CI is green | Pass | `gh run list -R una-lang/una --branch main`: run 36644973753 on `c2f3386` `completed success`, jobs `ubuntu-24.04` (3m6s) and `macos-26` (45s); macOS job log shows `Z3 version 5.1.0` and every `test result: ok`. Latest `dev/v0.1` run (0095fd5) also success; the `cancelled` runs are superseded pushes (`concurrency: cancel-in-progress`). |
| `una explain U0001` prints text | Pass | `una explain U0001` exit 0, prints `# U0001: Invalid character, byte order mark or lone carriage return` + paragraph + `## Failing example`. |
| `una check --json` prints empty list with schema `una.diagnostics/1` | Pass | `una --json check examples/hello` -> `{"schema":"una.diagnostics/1","diagnostics":[]}` exit 0 (same for all 7 examples). Outside a package it is now a usage error (exit 2, stderr only), as `spec/13 §7` requires. |
| Snapshot pins JSON shape of a sample diagnostic | Pass | `crates/una-diag/tests/json_shape.rs` `listed_diagnostic_shape_is_pinned` + `snapshots/json_shape__list_with_spec_example.snap` (U5001 with related spans); `cargo test -p una-diag --test json_shape`: 9 passed. |

Also verified (tasks): `scripts/check-solver.sh` prints `detected Z3 version 5.1.0 ... ok: z3 5.1.0 matches verification.lock`; every `crates/*/src/{lib,main}.rs` contains `forbid(unsafe_code)`.

## M1. Lexer and parser

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases for U0001 to U0007 and U0101 to U0103 | Pass | Files with `-- expect: error <code>`: U0001:6, U0002:2, U0003:3, U0004:3, U0005:2, U0006:1, U0007:4, U0101:7, U0102:7, U0103:2 (`tests/conformance/syntax/`, 47 cases). All pass in the runner. |
| Every `.una` file under `examples/` parses | Pass | `una-syntax --test examples` `every_example_parses_without_diagnostics ... ok`; `una check` exit 0 on all 7 packages. |
| Robustness: every prefix never panics, positions inside prefix, required boundaries yield a diagnostic (ADR 0017) | Pass | `crates/una-syntax/tests/robustness.rs:218` `every_prefix_of_every_example_is_parsed_safely ... ok`; the test covers every line boundary of every example, checks spans and repair edits `<= prefix.len()`, and requires a diagnostic after the first line of multi-line declarations and after the `=` line (asserts `>100` prefixes, `>20` required). |

## M2. Formatter

| Criterion | Status | Evidence |
|---|---|---|
| Idempotence for every `.una` file in the repository | Pass | `una-fmt --test properties` `every_file_is_formatted_idempotently_and_keeps_its_tree_and_comments ... ok`. Independent CLI run over the copy: 247 `.una` files, 206 formatted and idempotent (`fmt` twice, `cmp`), 0 non-idempotent, 41 refused (files with intended syntax errors). |
| Round trip `parse(fmt(x)) == parse(x)` modulo spans | Pass | Same test compares normalized tree dumps and comments (see Note N4 on normalization). |
| All examples are canonical | Pass | `una fmt --check examples/<each>` exit 0 for all 7; test asserts `canonical == text` for `examples/`. |

## M3. Project and names

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases for every listed code | Pass | U1001:6, U1002:7, U1003:1, U1004:1, U1005:3, U1006:1, U1007:1, U1008:1, U7002:3, U7004:2, U7005:2; all pass. |
| All examples resolve | Pass | `una check --tier 0` and tier 1 exit 0 on all 7 examples. |

## M4. Types

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases for U2001 to U2015 | Pass | Every code U2001..U2015 has 1 to 2 failing cases (e.g. U2001:2, U2007:1, U2015:2); all pass. |
| `examples/events/src/events/seating.una` passes tier 0 | Pass | `una check --tier 0 examples/events/src/events/seating.una` exit 0, no output. |

## M5. Capabilities, effects and failures

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases for U3001 to U3015 and U7003 | Pass | U3001..U3015 each 1 to 4 cases, U7003:1; all pass. |
| `una contract --json` snapshot for the events example | Pass | `crates/una-cli/tests/snapshots/cli__contract_check_in_json.snap` (cli.rs:551); `cd examples/events && una --json contract events.checkin.check_in` prints `{"schema":"una.contract/1","function":"events.checkin.check_in",...}`. Demo checkpoint 2 also checks inferred clauses in human and JSON. |

## M6. Information flow

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases for U4001 to U4007 (U4006 unassigned) | Pass | U4001:3, U4002:4, U4003:3, U4004:1, U4005:3, U4007:1. `spec/13 §4` line 160: U4006 is an unassigned number. |
| Classic implicit flow, failure under pc, labeled text to Console | Pass | `security/classic_implicit_flow.una` (U4001 at 8:6), `security/fail_under_secret_pc.una` (U4002 at 14:5), `security/labeled_text_to_console.una` (U4003 at 9:18). |
| Whole events example passes tier 0 | Pass | `una --json check --tier 0 examples/events` -> empty list, exit 0. |

## M7. Core IR and interpreter

| Criterion | Status | Evidence |
|---|---|---|
| `una test examples/events` passes | Pass | `6 examples: 6 passed, 0 failed`, exit 0. |
| `una run examples/hello --grant console` prints the greeting | Pass | Prints `Hello, Una`, exit 0. |
| Snapshot tests of Core dumps and trace JSON | Pass | `cli__core_events_{checkin,keys,main,seating,types}.snap` (test `dump_core_prints_the_core_ir_of_the_reference_example ... ok`); `runtime__test_events_json.snap`, `runtime__run_hello_json.snap`. |
| Redaction test: labeled arguments never appear in traces | Pass | `runtime.rs:172` `traces_never_show_labeled_arguments ... ok`. Independent: `una --json test examples/events` trace has `"data": "<redacted:secret>"`, `"key": "<redacted:internal>"`; the raw phone `12345678901` (checkin.una:56, :64) occurs 0 times in the output. |
| Differential check: every conformance `run-output` case matches | Pass | 34 `-- expect: run-output` cases, all pass in the runner (194/194). |

Also: `Keys` HMAC is RFC 2104 over `sha2` (`crates/una-interp/src/host.rs:207`) with RFC 4231 vectors 2 and 6 tested.

## M8. Verification

| Criterion | Status | Evidence |
|---|---|---|
| Conformance cases with proven, refuted (with counterexample) and unknown outcomes; U5001 to U5003 | Pass | `verification/`: 8 `*_proven` cases (`expect: ok`), 10 U5001 refuted cases, `step_budget_unknown` (U5002), `solver_version_mismatch` (U5003). Counterexamples are not asserted by the runner, but by `verification.rs` `a_refuted_obligation_shows_its_counterexample_and_repairs ... ok` and demo 4 (`counterexample: remaining = 0, result = Few`). |
| SMT queries for the examples byte-identical across runs (snapshot) | Pass | `verification.rs:139` + `verification__queries_events.snap` (events only, Minor M2). Independently for all 7 examples: cold `una check` with default threads and with `RAYON_NUM_THREADS=1` produce identical proof-cache key sets (keys are SHA-256 of the query text, `spec/12 §9`), with the same key set in both runs for every package. |
| Second run of `una check` reports every obligation as cached | Pass | For all 7 examples in the copy: clear `.una`, `una check`, then `una obligations`: events 11/11, greenhouse 19/19, journal 2/2, fifth example package 42/42, pantry 27/27, shortlinks 5/5 lines end `(cached)`; hello has 0. JSON second run: `cached: true` for all. See Note N1. |
| All examples verify with zero refuted and zero unknown | Pass | `una --json obligations` cold: events 11 proven, greenhouse 19, hello 0, journal 2, fifth example package 42, pantry 27, shortlinks 5; no refuted, no unknown; exit 0. |

Also: without Z3 on PATH and a cold cache, `una check examples/events` gives `[U5003] ... z3 5.1.0 is not available` exit 1; the conformance suite run with no Z3 fails 22 cases with U5003 (never skips).

## M9. Agent ABI and build report

| Criterion | Status | Evidence |
|---|---|---|
| Every `explain` example fails with its code and fixed version passes | Pass | `explain_examples.rs` `failing_examples_yield_their_code_and_fixed_examples_yield_nothing ... ok` (all codes but U8001 must have both). An independent reviewer script (tier 1 `check`, `run` for U6xxx, via the CLI): `64 codes with examples, 0 problems`; U8001 has no examples by design (`spec/13 §4` line 160). See Notes N2, N3. |
| Report and obligations JSON snapshots for the events example | Pass | `abi__report_events_json.snap` (abi.rs:48), `abi__obligations_events_json.snap` (abi.rs:112), plus `abi__trace_events_json.snap`; abi tests 8 passed. |

Also: `--json` probes on error paths all print one document on stdout with empty stderr (check, fmt --check, run rejected/failed, test, obligations, report, U7002, U5003); usage errors (unknown code, missing path, unknown function or requirement) exit 2 with stderr only, as `spec/13 §5` line 190 states.

## M10. Gate 1 readiness

| Criterion | Status | Evidence |
|---|---|---|
| Five additional example packages (the fifth example package alphabetically, plus pantry, journal, greenhouse, shortlinks) | Pass | Present; each checks, tests and runs (table below). `examples.rs` `the_examples_contain_every_construct_of_the_charter_scope ... ok` and `every_example_package_is_canonical_verified_and_tested ... ok`. |
| The guide's snippets compile | Pass | `guide.rs` `every_block_of_the_guide_is_checked ... ok`, `the_readme_shows_only_checked_code ... ok`. `docs/guide/una-for-agents.md` has 24 fenced blocks, all tagged (18 una, 3 json, 1 sh, 2 toml in packages); the test rejects untagged blocks outside packages. |
| Release artifacts build in CI | Partial | Linux: CI step `Pack and check the Linux release archive` passed on main run 36644973753. macOS arm64: only in `release.yml`, triggered by `v*` tags; `gh run list --workflow release.yml` is empty and no tag exists (tag is the owner's per ADR 0035). Locally `scripts/release.sh aarch64-apple-darwin` + `check-release.sh` in the copy: `ok: una-0.1.0-aarch64-apple-darwin.tar.gz checks and runs the examples`. Finding R-035. |
| Evaluation track can drive `una` headlessly through `--json` | Pass | `headless.rs` `an_agent_repairs_and_verifies_a_package_through_json_alone ... ok`; JSON probes above. |

## Examples and demos

Run in the copy with the `[run] grants` of each `una.toml` (no extra `--grant` needed).

| Package | `una check` | `una test` | `una run` | Output summary |
|---|---|---|---|---|
| events | 0 | 0 (6/6) | 0 | `availability of Launch: open` |
| greenhouse | 0 | 0 (3/3) | 0 | `vent at 66%` / `the warmer side asks for 66% open` |
| hello | 0 | 0 (0 examples) | 0 | `Hello, Una` |
| journal | 0 | 0 (3/3) | 0 | `today's entry runs to about 2 hundred characters`; no files left changed in the copy |
| (fifth example package) | 0 | 0 (7/7) | 0 | `Ana borrowed the ladder for 14 days` / `returned 20 days later, it is 6 days late` |
| pantry | 0 | 0 (7/7) | 0 | `to buy: rice, oil` / `grams in stock: 700` |
| shortlinks | 0 | 0 (6/6) | 0 | `docs leads to https://example.org/docs` / `blog leads to /` |

`una --json check` on each: `{"schema":"una.diagnostics/1","diagnostics":[]}`, exit 0.

| Script (run from the copy, `sh scripts/demo/checkpoint-N.sh target/debug/una`) | Exit | Result |
|---|---|---|
| checkpoint-1.sh (M1, M2; writes `target/demo/`) | 0 | 10 outcomes, 0 DIFFERENT, `every outcome is as expected` |
| checkpoint-2.sh (M3 to M6) | 0 | 11 outcomes, 0 DIFFERENT |
| checkpoint-3.sh (M7) | 0 | 11 outcomes, 0 DIFFERENT |
| checkpoint-4.sh (M8 to M10) | 0 | 10 outcomes, 0 DIFFERENT |

`demo_scripts.rs` `every_demo_script_finds_its_expected_outcomes ... ok` also runs them from the repo root.

## Conformance and determinism

- `CARGO_TARGET_DIR= cargo test -p una-cli --test conformance -- --nocapture` (Z3 5.1.0 on PATH): `conformance: 194 cases, 194 passed, 0 pending, 0 failures`; 29 unit tests of the runner pass; exit 0. The runner prints pending as a separate count; it is 0.
- Areas: authority 29, fmt 14, names 22, project 7, runtime 13, security 17, syntax 47, types 24, verification 21 entries. Expectations: 158 `error`, 9 `ok`, 34 `run-output`, 6 `test-fail`, 1 `test-pass`, 21 `tier`.
- Code coverage (`testing.md §5`): every code in `spec/13 §4` except U8001 has at least one failing case (64 codes).
- `grep -rn pending tests/`: no match; no `-- pending: M<n>` directive remains.
- Determinism tests: `cli.rs::check_json_is_byte_identical_across_runs`, `abi.rs::json_outputs_are_byte_identical_across_runs_and_thread_counts`, `verification.rs::a_second_run_reports_every_obligation_as_cached`, `verification.rs::the_queries_of_the_examples_are_byte_identical_across_runs`. Run with `cargo test -p una-cli --test {cli,abi,verification} -- byte_identical cached`: 1 + 1 + 2 passed; again with `RAYON_NUM_THREADS=1`: 1 + 1 + 2 passed.
- Independent: for every example, cold `una --json obligations` default vs cold with `RAYON_NUM_THREADS=1`: byte-identical; warm default vs warm threads=1: byte-identical.
- Without Z3 on PATH (copy, fresh target dir): conformance `194 cases, 172 passed, 0 pending, 22 failures`, each failure `U5003`, as `CLAUDE.md` requires (fail, never skip).

## Findings

### M1 (R-035). macOS arm64 release archive has never been built in CI
- Severity: Minor
- Reference: `plan/milestones-v0.1.md` M10 acceptance ("release artifacts build in CI"), ADR 0035, DECISION D-129.
- Evidence: `.github/workflows/ci.yml` packs only the Linux archive (`if: runner.os == 'Linux'`); `release.yml` runs only on `v*` tags; `gh run list -R una-lang/una --workflow release.yml` returns nothing and the repo has no tags. The local macOS build works (`check-release.sh`: `ok: ... checks and runs the examples`).
- Repro: `gh run list -R una-lang/una --workflow release.yml`; `gh api repos/una-lang/una/tags` -> `[]`.
- Recommended fix: run `release.yml` once via `workflow_dispatch` (or on a pre-release tag) before tagging `v0.1.0`, or record that the macOS half of the criterion is met only when the owner pushes the tag.

### M2 (R-036). SMT query snapshot covers only the events example
- Severity: Minor
- Reference: M8 acceptance ("SMT queries for the examples are byte-identical across runs (snapshot)"), `spec/12 §4`.
- Evidence: `crates/una-cli/tests/verification.rs:139` checks `examples/events` only; `verification__queries_events.snap` is the only query snapshot. My cache-key comparison shows the other six examples are deterministic today, but nothing pins them.
- Repro: `ls crates/una-cli/tests/snapshots | grep queries`.
- Recommended fix: loop the test over every package under `examples/` with one snapshot each.

### N1. `una check` itself does not report cache status
- Severity: Note
- Reference: M8 acceptance wording; `spec/13 §5` line 188.
- Evidence: `una check` prints nothing on success; cached status appears only in `una obligations`. `check` does populate the shared cache (all obligations `(cached)` right after a cold `check`).
- Recommended fix: none needed; reword the criterion to name `una obligations`.

### N2. The M9 explain check lives outside the conformance runner, and fixed examples run at tier 0
- Severity: Note
- Reference: M9 acceptance ("The conformance runner compiles every explain example").
- Evidence: the check is `crates/una-cli/tests/explain_examples.rs`, a separate test target; non-verification fixed examples are checked at tier 0 only (`explain_examples.rs:112-119`). My tier 1 CLI run found no problem in any of the 64 codes.
- Recommended fix: check fixed examples at tier 1, or accept as is and record the placement.

### N3. The check-JSON determinism test skips the verification area
- Severity: Note
- Reference: `compiler/testing.md §4`.
- Evidence: `cli.rs:109` iterates areas `syntax .. runtime` but not `verification`. `testing.md §4` only requires the examples, which it covers.
- Recommended fix: add `verification` to the list.

### N4. Round-trip property compares trees after a normalization
- Severity: Note
- Reference: M2 acceptance; ADR 0020 rules 4, 6, 7; DECISION D-030.
- Evidence: `crates/una-fmt/tests/properties.rs:122` `normalize` sorts imports and applies other canonical rewrites to both trees before comparing, so "equal modulo spans" is "equal modulo spans and those rewrites". This matches the ADR.
- Recommended fix: none; mention it in the criterion.

### N5. A warm proof cache answers without Z3
- Severity: Note
- Reference: `spec/12 §6` and §9 (version read only when a query misses the cache); `CLAUDE.md` rule on U5003.
- Evidence: with a warm `.una/cache`, `PATH=/usr/bin:/bin una check examples/events` exits 0. With a cold cache it gives U5003, and a cold test run without Z3 fails 22 cases with U5003. This follows the spec, but a developer's local test run can pass without Z3 because of committed-ignored caches from earlier runs.
- Recommended fix: none required; optionally clear per-package `.una/` caches in solver tests.

## After closure

Tree: `dev/v0.1` at the head of the closure pull request, after the v0.1 closure run (`docs/history/v0.1/runs/v0.1-closure-plan.md`). The commands ran on the owner's Mac (macOS, arm64) with Z3 5.1.0 from `scripts/install-z3.sh`. The adversarial programs ran against a copy of `adversarial/`.

**Summary:** 28 criteria; 28 pass. The Linux and macOS release archives are packed and checked in CI on the pull request (R-035). The regression tests of every Critical and Major finding are part of the suite. There are no pending conformance cases.

### Baseline (section 3.1 of the review)

| Command | Result |
| --- | --- |
| `cargo build --workspace --locked` | exit 0 |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --locked` | 348 passed, 0 failed, 0 ignored |
| `RAYON_NUM_THREADS=1 cargo test --workspace --locked` | 348 passed, 0 failed |
| Conformance runner | 374 cases: 374 passed, 0 pending, 0 failures |
| Conformance without Z3 on `PATH`, cold cache | 64 cases fail with U5003, and none are skipped, as `CLAUDE.md` requires |
| Determinism tests (`byte_identical`, `cached` in `cli`, `abi`, `verification`), default and with `RAYON_NUM_THREADS=1` | 1 + 1 + 4 passed in both runs |
| Cold `una --json obligations`, default and with `RAYON_NUM_THREADS=1` | byte-identical for all 7 examples |
| `cargo doc --workspace --no-deps --document-private-items` | 0 warnings (6 at review, R-039) |
| Examples: `una check`, `una test`, `una run` | all 7 exit 0 for all three commands; tests: events 6/6, greenhouse 3/3, hello 0, journal 3/3, pantry 7/7, shortlinks 6/6, toolshelf 7/7 |
| `scripts/demo/checkpoint-1.sh` to `-4.sh`, with an absolute path to `una` | all exit 0, 42 outcomes, none different |

### Audits

| Audit | Result (counts only) |
| --- | --- |
| Naming, whole word | hits only in the files ADR 0025 allows: ADR 0016, ADR 0025, `docs/reviews/triage.md` and the history files under `docs/history/v0.1/` |
| Code shape `[VT][0-9]{4}` | hits only in ADR 0016 and two history files |
| Neutrality (R-022) | 0 hits outside the history and review files; 9 lines inside them (`docs/history/` 7, `docs/reviews/v0.1-review/` 1, `docs/reviews/triage.md` 1); 0 file or directory names |

### The adversarial programs

All 143 programs were rerun with `una check`. 127 are rejected. The 16 that are accepted are the valid programs of the review's table: controls, runtime programs whose run behavior is the point, and R-019's program, which must now pass:
`auth-fn-param-without-uses.una`, `flow-main-failure-payload.una`, `flow-policy-method-call`, `runtime-checked-environment`, `runtime-deep-recursion-memory`, `runtime-euclidean-division`, `runtime-evaluation-order`, `runtime-files-dangling-link`, `runtime-host-trace-redaction`, `runtime-prelude-contracts`, `runtime-short-circuit-interpolation`, `runtime-test-result-unredacted`, `runtime-trace-redaction`, `verify-host-capability-shadow`, `verify-range-overlap-first-wins`, `verify-shared-field-name-sorts`.

The Critical findings:

| Program | Finding | Exit | Codes |
| --- | --- | --- | --- |
| `auth-escape-failure.una` | R-001 | 1 | U3009 U3016 |
| `auth-escape-through-generic.una` | R-002 | 1 | U3016x3 |
| `auth-escape-to-pure.una` | R-001 | 1 | U3003x2 U3016x3 |
| `auth-uses-dropped-in-list.una` | R-001 | 1 | U3003 U3016 |
| `escape-policy-opaque-fact` | R-007 | 1 | U5001 |
| `flow-generic-equality.una` | R-004 | 1 | U4001 U4003 |
| `flow-generic-filter-count.una` | R-004 | 1 | U4001x2 |
| `flow-generic-first-where.una` | R-004 | 1 | U4001x3 |
| `flow-handler-pc.una` | R-003 | 1 | U4003 |
| `flow-prelude-contains.una` | R-005 | 1 | U4001 U4003 |
| `flow-secret-fn-param-call.una` | R-006 | 1 | U4002 |
| `flow-secret-fn-value-effect.una` | R-006 | 1 | U3016x2 U4002x2 |
| `runtime-files-dangling-link` | R-011 | 0 at check; `una run` after its setup prints `false` and creates nothing outside the grant | none |
| `verify-host-capability-name-confusion` | R-008 | 1 | U5001 |
| `verify-policy-to-refinement` | R-007 | 1 | U5001 |
| `verify-prelude-module-variants` | R-010 | 1 | U1009 |
| `verify-tag-field-collision` | R-009 | 1 | U5001 |

The Major findings with a program:

| Program | Finding | Exit | Codes |
| --- | --- | --- | --- |
| `escape-opaque-predicate-in-if` | R-016 | 1 | U2016 |
| `escape-proof-cache-forgery` | R-018 | 1 | U5001 |
| `escape-testing-via-handler` | R-017 | 1 | U3015x2 |
| `flow-hmac-without-policy.una` | R-014 | 1 | U4001 |
| `flow-host-op-fails-on-secret.una` | R-015 | 1 | U4003 |
| `verify-fn-type-refinement-erased` | R-013 | 1 | U5001x2 |
| `verify-generic-sum-refinement` | R-012 | 1 | U2002 |
| `verify-refinement-type-arguments.una` | R-012 | 1 | U2002x2 |
| `verify-shared-field-name-sorts` | R-019 | 0 | none |

Where each program lives in the test suite:
- 140 programs are conformance cases or CLI tests. The table is in the closure plan's progress section, and each case's `-- test:` line names its finding or says `review control`.
- 3 are not promoted, because their findings are deferred to the backlog: `verify-negative-literal-operands` (R-031), `runtime-deep-recursion-memory` (R-033) and `runtime-test-result-unredacted` (R-025).

### Acceptance criteria, M0 to M10

| Milestone | Criterion | Status | Evidence |
| --- | --- | --- | --- |
| M0 | `cargo test --workspace` passes | Pass | 348 passed (above) |
| M0 | CI is green | Pass | pull request #1, CI run 36759795381 on `c12a72f`: `ubuntu-24.04` and `macos-26` pass (Z3 5.1.0, every test binary ok) |
| M0 | `una explain U0001` prints text | Pass | `# U0001: Invalid character, byte order mark or lone carriage return` |
| M0 | `una check --json` prints an empty list with schema `una.diagnostics/1` | Pass | `una --json check examples/hello` |
| M0 | a snapshot pins the JSON shape | Pass | `json_shape.rs`, in the suite |
| M1 | conformance cases for U0001 to U0007 and U0101 to U0103 | Pass | failing cases: U0001 7, U0002 2, U0003 3, U0004 3, U0005 2, U0006 1, U0007 4, U0101 8 (with B-001), U0102 7, U0103 2 |
| M1 | every `.una` file under `examples/` parses | Pass | `every_example_parses_without_diagnostics` |
| M1 | robustness over every prefix | Pass | `every_prefix_of_every_example_is_parsed_safely` |
| M2 | idempotence, round trip | Pass | `crates/una-fmt/tests/properties.rs` |
| M2 | all examples are canonical | Pass | `una fmt --check` exit 0 for all 7 |
| M3 | conformance cases for every listed code | Pass | U1001 7, U1002 7, U1003 1, U1004 1, U1005 3, U1006 1, U1007 1, U1008 1, U1009 2 (new), U7002 3, U7004 2, U7005 4 |
| M3 | all examples resolve | Pass | `una check --tier 0` exit 0 for all 7 |
| M4 | conformance cases for U2001 to U2015 | Pass | each has 1 to 5 failing cases, and U2016 (new) has 2 |
| M4 | `seating.una` passes tier 0 | Pass | exit 0 |
| M5 | conformance cases for U3001 to U3015 and U7003 | Pass | each has 1 to 13 failing cases; U3016 (new) has 8; U7003 4 |
| M5 | `una contract --json` snapshot | Pass | `cli__contract_check_in_json.snap` |
| M6 | conformance cases for U4001 to U4007 | Pass | U4001 49, U4002 14, U4003 13, U4004 2, U4005 6, U4007 1 |
| M6 | classic implicit flow, failure under pc, labeled text to Console | Pass | `security/classic_implicit_flow`, `fail_under_secret_pc`, `labeled_text_to_console` |
| M6 | the events example passes tier 0 | Pass | `una --json check --tier 0 examples/events`: empty list |
| M7 | `una test examples/events` passes | Pass | 6 examples: 6 passed |
| M7 | `una run examples/hello --grant console` | Pass | prints `Hello, Una` |
| M7 | snapshots of Core dumps and trace JSON; redaction test | Pass | in the suite, with the redaction CLI tests of the adversarial runtime programs |
| M7 | every `run-output` case matches | Pass | conformance, 374 of 374 |
| M8 | proven, refuted and unknown cases; U5001 to U5003 | Pass | U5001 40 cases, U5002 1, U5003 1, and 8 new function-type cases (ADR 0037) |
| M8 | SMT queries byte-identical (snapshot) | Pass | one query snapshot per example package (R-036) |
| M8 | a second run reports every obligation as cached | Pass | events 11/11, greenhouse 19/19, journal 2/2, pantry 27/27, shortlinks 5/5, toolshelf 42/42, hello 0 |
| M8 | all examples verify with zero refuted and zero unknown | Pass | every obligation `proven` in all 7 |
| M9 | every explain example fails with its code, and its fixed version passes | Pass | `explain_examples.rs`, now at tier 1 for fixed examples (R-037) |
| M9 | report and obligations JSON snapshots | Pass | `abi__report_events_json.snap` (with `obligations.cached`, ADR 0041), `abi__obligations_events_json.snap` |
| M10 | five more example packages | Pass | `greenhouse`, `journal`, `pantry`, `shortlinks`, `toolshelf` (renamed, R-022) |
| M10 | the guide's snippets compile | Pass | `guide.rs`, both tests |
| M10 | release artifacts build in CI | Pass | CI run 36759795381: `una-0.1.0-x86_64-unknown-linux-gnu.tar.gz` and `una-0.1.0-aarch64-apple-darwin.tar.gz` packed and checked, each running the examples (R-035) |
| M10 | the evaluation track drives `una` headlessly | Pass | `headless.rs` |

The review's recommendation to treat M5, M6 and M8 as not done until the Critical findings were fixed is met. Each Critical finding has a rule in ADR 0038 and regression cases in the suite, and each program above is rejected.
