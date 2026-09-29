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

## 7. Self-review checklist

Appended when the milestone is complete.
