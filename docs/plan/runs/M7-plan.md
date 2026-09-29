# M7 plan: Core IR and interpreter

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/09`, `spec/11`, `spec/14`, `spec/08 §2`, ADR 0002, ADR 0004; also `spec/13 §5` (commands, output streams), `spec/04 §8` (entry point), `spec/05 §7` (grants), ADR 0015 (output streams), ADR 0022 (diagnostics without a source position) and ADR 0027 (interim run behavior).

Codes: U3012 at startup (M5 reports it from `[run] grants` in `una check`), U6001, U6003. Acceptance (`plan/milestones-v0.1.md`): `una test examples/events` passes; `una run examples/hello --grant console` prints the greeting; snapshot tests of Core dumps and trace JSON; a redaction test proves that labeled arguments never appear in traces; every conformance `run-output` case matches. M7 completes usage checkpoint 3, "Run" (`plan/roadmap.md §4`), whose demo comes with it.

## 1. Design

- **Crates** (`compiler/architecture.md §2`): `una-core` holds Core IR, its printer and its values' shapes; `una-check` elaborates checked declarations to Core; `una-interp` evaluates Core, with host capabilities behind a `World` trait that `una-cli` implements with real input and output, so the library crates never print (`CLAUDE.md`). `una-core` uses the spans of `una-syntax`, since every Core node keeps its source span (`spec/10`, preamble).
- **Elaboration from the checker's resolutions.** Pass 2 of the checker records what it resolves, by the span of each expression: the callee of each call and its type arguments, the parameter each argument goes to, the field or operation of each access, the variant of each constructor and pattern, the capability and handler of each binding, and the labeled type of each expression and binder. The elaborator walks the syntax tree with these tables and applies the desugaring table of `spec/09 §3`: A-normal form with `%` temporaries, positional arguments, `and`, `or`, `implies` and `in` as `if`, `when` as `case` or `if` chains, handling clauses as `handle` and `case`, `with` nesting, interpolation as `concat` and `to_text`, policy calls as `declassify`, statements as `let _: Unit`. Only declarations without diagnostics are elaborated (`compiler/architecture.md §3`).
- **The evaluator** implements the UAM of `spec/11` directly over Core: big-step, call-by-value, left to right; arbitrary-precision `Int` (`num-bigint`, ADR 0004) with Euclidean `div` and `mod`; environments, a capability environment of instances with stable ids, a world, and traces. The prelude's functions are implemented natively (`spec/14 §1`); predicates run their bodies. A call depth above the limit (10,000 by default, `--max-depth`) aborts with U6003 (`spec/11 §7`).
- **Host capabilities** (`spec/14 §3`): `Console.print` writes a line; `Clock.now` gives milliseconds since the Unix epoch; `Env.get` reads a variable; `Files.read` and `Files.write` resolve paths against the grant's prefix and mode and fail with `FileError.Denied` outside them; `Keys.hmac` computes HMAC-SHA-256 over `sha2` with a key from the grant's key file. The world performs the input and output; the interpreter decides confinement.
- **Traces** (`spec/11 §5`): one event per operation, with its sequence number, instance, capability, operation, arguments by parameter name, redacted by their static labels, and outcome; handler events precede the events of the handler's body.
- **`una run`** checks the package at tier 0 (ADR 0027), compares `main`'s host capabilities with the grants of `[run] grants` and `--grant` (U3012, exit 1 before running), then evaluates `main` of `[run] entry` with the granted host instances. A failure of `main` is U6001; its payload fields are shown only when their labels are `public` (`spec/04 §8`).
- **`una test`** checks the package at tier 0, then runs every example in a fresh machine: `given` lines install fresh instances, the `do` line is evaluated, and the expectations are checked against its result and trace (`spec/08 §2`, `spec/11 §6`).
- **Conformance** gains the `runtime` area, and the runner evaluates `run-output`, `test-pass` and `test-fail` in-process.

## 2. Spec gaps and ADR 0032

ADR 0032 (Provisional), "runtime details the specification leaves open", records:
1. **Elaboration** (D-098): from side tables that pass 2 records; `una-core` depends on `una-syntax` for spans; only declarations without diagnostics are elaborated.
2. **The Core dump** (D-099): `una check --dump-core <module> [path]` prints the module's declarations in the text syntax of `spec/09 §2`, one declaration per block, with labels and refinement names on binder types; diagnostics still go to stderr.
3. **Values in traces and outputs** (D-100): every value is rendered as a JSON string holding its Una form (`"42"`, `"\"Ana\""`, `"Open"`, `"Some(3)"`, `"unit"`), and a `Hashed` value as `"hashed:<hex>"`; labeled arguments are `"<redacted:level>"`, the label's level only.
4. **`una run`** (D-101): the program's printed lines go to stdout, U6001 and U6003 to stderr as diagnostics; `--json` prints `una.run/1` with `tier`, `status`, the printed lines, the failure, the diagnostics and the trace. U6001 points at `main`'s name, U6003 at the call that exceeds the depth.
5. **`una test`** (D-102): one line per example and a summary in human mode; `--json` prints `una.test/1` with `tier` and, per example, its module, name, outcome, the failed expectation with its position, the result or failure, and the trace.
6. **Grants** (D-103): the grants of `[run] grants` and of `--grant` flags add up; relative paths of `una.toml` grants resolve against the package root, those of flags against the current directory; `Files` paths are relative to the prefix, normalized lexically and then through the file system for existing parts, and any path outside the prefix, absolute or through a link, is `Denied`; the key file is a TOML table of names to base64 keys, and an unknown name is `KeyError.UnknownKey`.
7. **The prelude's semantics** (D-104): `.len` of `Text` counts Unicode scalar values; `is_digits` holds when every character is an ASCII digit, so it holds of the empty text; `trim` removes Unicode whitespace at both ends; `to_int` reads an optional `-` and ASCII digits; `Text` orders by scalar values; equality is structural.
8. **Depth** (D-105): the depth counts calls of functions, predicates, lambdas, policies and handler operations; the evaluator runs on a thread with a stack large enough for the default limit.

Added while implementing (the ADR records them as rules 9 to 11, and refines rules 1 and 3 to 6 and 8):

9. **Examples** (D-106): a `do` line that fails with no `expect fails` line fails the example; `expect` lines without `result` run even then; `performed` counts completed or failed events.
10. **Entry and startup** (D-109): the entry of a single file is its module; `una run` checks with the grants of `una.toml` and `--grant` together as final grants, so U3012 covers flags; bad entries, grants and key files are usage errors.
11. **Faults** (D-108): a division by zero before tier 1 aborts with U8001.

Also D-107 (labeled record fields and labeled results are redacted) and D-110 (how the conformance runner evaluates `run-output`, `test-pass` and `test-fail`).

## 3. Tasks and commits

1. **Plan and ADR 0032** (`docs(plan)`).
2. **Core IR** (`feat(core)`): types, printer.
3. **Elaboration** (`feat(check)`): side tables, the elaborator, `una check --dump-core`, Core snapshots of the reference example.
4. **Evaluator** (`feat(interp)`): values, the UAM rules, the prelude, traces, the example harness, depth.
5. **Host capabilities and `una run`** (`feat(interp)`, `feat(cli)`): the world, `Files` confinement, `Keys` HMAC, grants, U3012, U6001, U6003, JSON.
6. **`una test`** (`feat(cli)`): the harness, human and JSON output, the redaction test and trace snapshots.
7. **Conformance, explain and demo** (`test(runtime)`, `docs(diag)`): `runtime` cases with `run-output`, `test-pass` and `test-fail`, the runner's evaluation, explain texts for U6001 and U6003, the checkpoint 3 demo.
8. **Self-review** (`docs(plan)`).

## 4. Conformance cases (`tests/conformance/runtime/`)

`run-output` cases for arithmetic with large integers and Euclidean division, text interpolation, lists and the prelude's higher-order functions, `when` over variants and ranges, failures and their handling, `attempt`, handlers installed with `with`, and `checked`; `test-pass` and `test-fail` cases for each expectation form; a program whose `main` fails.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| `una test examples/events` passes | a CLI test |
| `una run examples/hello --grant console` prints the greeting | a CLI test |
| Snapshots of Core dumps and trace JSON | insta snapshots of the reference example |
| Labeled arguments never appear in traces | a redaction test |
| Every conformance `run-output` case matches | runner summary |
| Checkpoint 3 demo | `scripts/demo/checkpoint-3.sh` |

## 6. Known gaps

- Tier 1 (proofs) arrives in M8; until then `una run` and `una test` check at tier 0 and say so (ADR 0027).
- `una replay` of recorded host results is in the backlog (`spec/11 §5`).
- Values nested deeper than the evaluator's stack, which a fold can build without deep calls, can still exhaust the stack when they are compared, rendered or dropped. Interpreter stack exhaustion is outside the noninterference model (`spec/07 §1`); the depth guard covers calls only (D-105).
- Trace parameter names come from the first capability of a given name, in module path order; two modules that declare capabilities with the same name and different parameter names share the names of the first.

## 7. Self-review checklist

Every MUST and MUST NOT of the milestone's sections, then the other rules they state. Evidence names tests of `crates/una-cli/tests/runtime.rs` (R), unit tests of `crates/una-interp` (U), cases of `tests/conformance/runtime/` (C), and the checkpoint 3 demo (D).

### MUST and MUST NOT

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/11 §1`: a backend's results and traces MUST be ones the UAM can produce | done: the v0.1 interpreter is the UAM, rule by rule over Core IR | C every case; R every test |
| `spec/08 §2`: `expect e` MUST evaluate to `true` | done, with the first expectation that does not hold reported (D-106) | C `examples_pass`, `examples_fail`; R `test_reports_the_expectations_that_do_not_hold` |
| `spec/08 §2` rule 1: examples MUST NOT use host capabilities (U3014) | done in M5; examples run with an empty world (`spec/11 §6`) | M5 cases; `NoWorld` in `host.rs` |
| `spec/04 §8`: `main` MAY declare `uses` over host capabilities and `fails` | done; the entry must be `pub main() -> Unit` (D-109) | R `run_reports_bad_grants_entries_and_mains_as_usage_errors`, `a_failing_main_is_u6001_and_shows_only_public_fields` |
| `spec/14 §3`: user code MAY write handlers for host capabilities; host implementations are used only by `una run` | done: host instances exist only for the `uses` of `main` | the reference example's `fake_keys`; R `test_runs_the_examples_of_the_reference_example` |

### Other rules of the sections

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/09 §1` to `§4`: Core IR, its desugaring and `--dump-core` | done in `59a481d` (D-098, D-099) | Core snapshots `cli__core_events_*`; D |
| `spec/11 §2`: runtime objects; instances have stable ids and rebinding shares them | done | R `handler_instances_keep_their_ids_and_come_before_their_bodies` |
| `spec/11 §4`: E-Let, E-LetFail, E-Call with `H` restricted to the callee's `uses`, E-OpHost, E-OpHandler, E-Rebind, E-Handler, E-Fail, E-Attempt, E-HandleFail, E-CheckOk, E-CheckFail, `declassify` through `via`, Euclidean `div` and `mod` | done | U `division_is_euclidean`; C `big_integers`, `euclidean_division`, `failures_and_handling`, `handlers`, `checked`, `when_arms`; the reference example's policy |
| `spec/11 §5`: one event per operation; instance ids; arguments redacted above `public`; handler events first; host results recorded, subject to redaction | done, with labeled fields and results redacted too (D-100, D-107) | R `traces_never_show_labeled_arguments`, `keys_grants_load_their_key_file`, `json_documents_carry_the_tier_and_no_note` (snapshot) |
| `spec/11 §6`: each example in a fresh machine, `given` lines as fresh instances, empty environment and world | done | C `examples_pass`; R snapshot `test_events_json` |
| `spec/11 §7`: maximum call depth, 10,000 by default and `--max-depth`, U6003 | done, and the stack cannot overflow first (D-105) | U `calls_stop_at_the_maximum_depth_or_when_the_stack_runs_short`; R `a_call_deeper_than_the_limit_is_u6003`; C `call_depth`; D |
| `spec/14 §1`: the prelude's functions | done natively (D-104) | C `prelude_functions` |
| `spec/14 §2`: `.len`, arithmetic, comparisons, equality and interpolation | done; `.len` counts scalar values | C `text_interpolation`, `big_integers` |
| `spec/14 §3`: `Console`, `Clock`, `Env`, `Files` confined to the grant's prefix with `Denied` outside, `Keys` with HMAC-SHA-256 and the grant's key file | done (D-103) | U `hmac_matches_rfc_4231`, `key_files_are_tables_of_base64_keys`; R `files_grants_confine_paths_to_their_prefix`, `keys_grants_load_their_key_file`, `grants_are_checked_before_running` |
| `spec/14 §4`: `fake_hash` gives equal values for equal seeds | done: SHA-256 of the seed | the reference example's examples |
| `spec/04 §8`: U3012 before execution; a failing `main` prints its variant with exit code 1 and only its `public` fields | done (D-109) | C `startup_grants`, `main_fails`; R `grants_are_checked_before_running`, `a_failing_main_is_u6001_and_shows_only_public_fields`; D |
| `spec/13 §5`: `una run` and `una test`, `--grant`, program output on stdout, only JSON on stdout with `--json` | done (D-101, D-102) | R `run_prints_the_greeting`, `json_documents_carry_the_tier_and_no_note` |
| ADR 0027: tier 0 before running, the note in human mode, `"tier": 0` with `--json` | done | R `run_and_test_do_not_run_rejected_packages`, `json_documents_carry_the_tier_and_no_note` |

### Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| `una test examples/events` passes | R `test_runs_the_examples_of_the_reference_example` |
| `una run examples/hello --grant console` prints the greeting | R `run_prints_the_greeting` |
| Snapshots of Core dumps and trace JSON | `cli__core_events_*.snap`; `runtime__run_hello_json.snap`, `runtime__test_events_json.snap` |
| Labeled arguments never appear in traces | R `traces_never_show_labeled_arguments` |
| Every conformance `run-output` case matches | runner: 173 cases, 172 passed, 1 pending (M8), 0 failures |
| Checkpoint 3 demo | `scripts/demo/checkpoint-3.sh`, run by `crates/una-cli/tests/demo_scripts.rs` |
| Explain texts for U6001 and U6003 | `crates/una-cli/tests/explain_examples.rs`, which runs their examples with `una run` |

### Found during the milestone

- A record whose own label is `public` can hold a `secret` field, which argument redaction alone would print; every rendered value now redacts labeled fields (D-107).
- A fixed estimate of stack per call was not enough for recursive functions with nested expressions in unoptimized builds; the evaluator now measures its stack and aborts with U6003 before it runs short (D-105).
- `una check` compared `main` with `[run] grants` only, which would have rejected a `--grant` flag; `una run` now checks with its final grants (D-109).
- The library must not read environment variables (`compiler/conventions.md §1`), so the binary passes them to `una_cli::run` for `Env.get`.
