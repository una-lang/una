# 0013. Conformance directives: well-formedness, multi-module cases and pending cases

- Status: Accepted
- Date: 2026-09-28
- Deciders: Owner
- Amended: 2026-09-28, D6 (Deciders: Owner)

## Context

`compiler/testing.md §2` defines conformance cases and their directives, and says that the runner "discovers cases, runs the compiler in-process and compares results. A case whose directives are malformed fails." Milestone M0 builds that runner (`plan/milestones-v0.1.md`, M0 task 4). Three gaps prevent a runner that is both strict and green:

1. **Cases that cannot pass yet.** The three cases present at the start of the project expect U3005 (`authority/cap_as_value.una`, due in M5), U4002 (`security/fail_under_secret_pc.una`, M6) and U5001 (`verification/postcondition_refuted.una`, M8). They fail by construction until those milestones. testing.md has no way to say so: either `cargo test --workspace` stays red until M8, or the runner tolerates failures and hides regressions.
2. **"Malformed" is undefined.** testing.md does not say where the directive block ends, which directives are required or repeatable, which values are valid, or how quoted text is escaped.
3. **Multi-module cases.** testing.md allows a directory with `una.toml` and `src/`, but does not say where its directives are written (`una.toml` is TOML, which has no `--` comments), nor which file an `error ... at <line>:<col>` position refers to. Some errors are located outside `.una` files (U7002 in `una.toml`).

## Decision

D1 adds a directive. D2 to D5 only fill gaps in testing.md §2: they change none of its existing directive semantics.

### D1. The `pending` directive

`-- pending: M<n>` declares that the case is expected to fail until milestone `M<n>` of `plan/milestones-v0.1.md`, with `n` from 1 to 10. A case has at most one. The runner validates every directive of a pending case and runs it like any other case, then classifies it:

| `pending` | Outcome matches all expectations | Result |
| --- | --- | --- |
| absent | yes | pass |
| absent | no | fail |
| present | no | pending: listed in the runner summary, does not fail the run |
| present | yes | fail, with the message `pending case passes; remove -- pending: M<n>` |

An expectation the compiler cannot evaluate yet (for example `run-output` before `una run` exists) does not match. A milestone is complete only when no case names it in `pending`, and no case may be pending at release (M10).

### D2. The directive block

1. The directives of a file are its leading lines of the form `-- <name>: <value>`: no indentation, one space after `--`, one space after the colon, no trailing whitespace. The block ends at the first line of any other form.
2. The names are `test`, `spec`, `expect`, `grant` and `pending`. Any other name inside the block is malformed. An unindented line starting with `-- <name>: ` for one of these names after the block is malformed, so a misplaced directive is never silently ignored.
3. The runner reads cases as bytes and requires the directive block to be valid UTF-8. A leading byte order mark and `\r\n` line endings do not affect directive parsing, so that cases for U0001 can be written.

### D3. Values and cardinality

| Directive | Per case | Value |
| --- | --- | --- |
| `test` | exactly 1 | non-empty text |
| `spec` | exactly 1 | `spec/NN §a`, optionally followed by `, §b` and further sections; the file `docs/spec/NN-*.md` and a heading `## a.` for each section MUST exist |
| `expect` | at least 1 | one of the forms below |
| `grant` | any number | a grant spec of `spec/13 §5` |
| `pending` | at most 1 | `M1` to `M10` |

Expectation forms:

- `ok`, `tier0-ok` and `test-pass`: at most once each.
- `error <code> at <line>:<col>`: `<code>` is registered in `spec/13 §4`; `<line>` and `<col>` are 1-based decimal integers, the column counted in Unicode scalar values, and they MUST denote a position of the file: an existing line, and a column at most one past its last character. Two identical `error` lines are malformed, because expectations form a set.
- `run-output "<text>"`: one directive per printed line, in order; repeats are allowed.
- `test-fail "<example>"`: a non-empty example name; two identical lines are malformed.

Quoted values accept `\"` and `\\` as their only escapes. `ok`, `tier0-ok` and `error` are mutually exclusive, and so are `test-pass` and `test-fail`.

### D4. Multi-module cases

A directory case is a directory inside an area that contains `una.toml`. Exactly one of its `.una` files under `src/` carries the directive block (D2); the other files MUST NOT contain directives. In a directory case, `error` expectations name the file: `error <code> at <path>:<line>:<col>`, where `<path>` is relative to the case directory (for example `src/a/b.una` or `una.toml`), MUST exist, and MUST NOT contain `:`. Single-module cases keep `at <line>:<col>`.

### D5. Discovery

Every directory directly under `tests/conformance/` MUST be one of the areas of testing.md §2; files there (such as `README.md`) are not cases. Inside an area, every `.una` file is a single-module case and every directory containing `una.toml` is a multi-module case; any other entry is malformed. A run that discovers no case fails.

### D6. Hidden entries (amendment)

The runner ignores every entry whose name starts with `.`: at the conformance root, inside areas, and among the files of a multi-module case. Files that tools create, such as `.DS_Store`, are therefore never cases and never malformed. D6 amends D5.

On acceptance, testing.md §2 is updated with D1 to D5. In the same change as the runner, the three existing cases gain `-- pending: M5`, `-- pending: M6` and `-- pending: M8` at the end of their directive blocks. The added line shifts the code of each case down by one line, so their expected positions become `13:11`, `15:5` and `11:3`; they still point at the same tokens.

## Consequences

- `cargo test --workspace` is green from M0 while every case is still parsed and run, so malformed directives and unexpected passes are caught immediately.
- Each case records when it is due; searching for `-- pending: M5` lists what M5 must make pass.
- The strict rule of D1 forces removing a marker in the change that makes the case pass.
- Cases carry a development-only directive until M10.
- Directive validation depends on `docs/spec/` headings and on the code registry, so renumbering a spec section breaks the cases that cite it. This is intended.

## Alternatives considered

- **A pending list in the runner or in a manifest file:** keeps case files unchanged, but separates the marker from the case and is easy to forget when cases move.
- **Enabling whole areas per milestone:** coarse, and cases would silently not run; an area case that also needs a later milestone could not be expressed.
- **Marking pending cases by file name or directory:** renames files when they start passing, which breaks history and the rule of naming a file after its rule (`tests/conformance/README.md`).
- **Non-strict pending, where a passing pending case is not an error:** simpler, but markers would outlive the features they wait for.
- **Directives spread over each file of a multi-module case, with positions relative to their own file:** needs no path syntax, but cannot express errors located in `una.toml` or `verification.lock`.
- **Directives of multi-module cases in `una.toml` comments or a separate file:** keeps them at the case root, but needs a second comment syntax or file format.
- **Keeping hidden files malformed (D5 before D6):** strict, but any file created by the operating system inside an area fails the suite locally while CI stays green.
