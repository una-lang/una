# Testing strategy

Status: Normative for the reference implementation.

## 1. Layers

| Layer | Location | Purpose |
| --- | --- | --- |
| Unit | `crates/*/src/**` | Small functions: lexer states, unification, formula normalization |
| Conformance | `tests/conformance/` | One file per spec rule, positive and negative |
| Snapshot | `crates/*/tests/snapshots/` (insta) | Full JSON diagnostics, Core IR dumps, SMT queries, reports |
| Property | `crates/una-fmt/tests/` | Formatter idempotence and parse round-trip over every `.una` file in the repository |
| End to end | `crates/una-cli/tests/` | The CLI and the ABI documents, the runtime, verification, determinism, the explain examples, the checkpoint demos, the example packages and the guide |

## 2. Conformance case format

Single-module cases are one file, compiled as a package containing only that module, with default `una.toml` settings and without the module path check (U1008). Multi-module cases are a directory with `una.toml` and `src/`, or, for edition `0.4`, with `ita.una`, `ita.lock` and `src/` (ADR 0122).

```text
tests/conformance/<area>/<case>.una
tests/conformance/<area>/<case>/una.toml
```

Areas: `syntax`, `fmt`, `names`, `types`, `authority`, `security`, `verification`, `runtime`, `examples`, `project`.

Each case starts with directive comments:

```una
-- test: capability reference cannot be bound with let
-- spec: spec/05 §6
-- expect: error U3005 at 7:13
module conformance.authority.cap_let

...
```

| Directive | Meaning |
| --- | --- |
| `-- test: <text>` | Required. One-line description |
| `-- spec: <ref>` | Required. The rule under test |
| `-- expect: ok` | No diagnostics at tier 1 |
| `-- expect: error <code> at <line>:<col>` | Repeatable. The set of reported errors MUST equal the set of expect lines |
| `-- expect: tier0-ok` | No diagnostics at tier 0; tier 1 not run |
| `-- expect: run-output "<text>"` | `una run` prints exactly this (repeatable, one per line) |
| `-- expect: test-pass` / `-- expect: test-fail "<example>"` | Outcome of `una test` |
| `-- grant: <spec>` | Grant passed to `una run` |
| `-- pending: M<n>` | Optional. The case is expected to fail until milestone `M<n>` (see "Pending cases") |

The runner (`cargo test -p una-cli --test conformance`) discovers cases, runs the compiler in-process and compares results. It runs `una run --json` when a case expects output, grants a capability or expects an error of the runtime domain, and `una test --json` when it expects `test-pass` or `test-fail`; the errors these commands report join those of `una check` for `error` expectations, and the relative paths of `files:` and `keys:` grants resolve against the case's directory. A case whose directives are malformed fails. The rules below define well-formedness and pending cases (ADR 0013); they fill gaps and change none of the meanings above.

### Directive block

1. The directives of a file are its leading lines of the form `-- <name>: <value>`: no indentation, one space after `--`, one space after the colon, no trailing whitespace. The block ends at the first line of any other form.
2. The names are `test`, `spec`, `expect`, `grant` and `pending`. Any other name inside the block is malformed. An unindented line starting with `-- <name>: ` for one of these names after the block is malformed, so a misplaced directive is never silently ignored.
3. The runner reads cases as bytes and requires the directive block to be valid UTF-8. A leading byte order mark and `\r\n` line endings do not affect directive parsing.

### Values and cardinality

| Directive | Per case | Value |
| --- | --- | --- |
| `test` | exactly 1 | non-empty text |
| `spec` | exactly 1 | `spec/NN §a`, optionally followed by `, §b` and further sections; the file `docs/spec/NN-*.md` and a heading `## a.` for each section MUST exist |
| `expect` | at least 1 | one of the forms in the table above |
| `grant` | any number | a grant spec of `spec/13 §5` |
| `pending` | at most 1 | `M1` to `M10` |

- `ok`, `tier0-ok` and `test-pass` appear at most once each.
- In `error <code> at <line>:<col>`, `<code>` is registered in `spec/13 §4`; `<line>` and `<col>` are 1-based decimal integers, the column counted in Unicode scalar values with the rule of `spec/13 §1` (ADR 0015), and they MUST denote a position of the file: an existing line, and a column at most one past its last character. Two identical `error` lines are malformed, because expectations form a set.
- A single-module case is its own package root (ADR 0015), so its expected errors match diagnostics whose `file` is the case's file name, for example `cap_as_value.una`.
- `run-output` directives are compared in order, one per printed line; repeats are allowed.
- `test-fail` names a non-empty example; two identical `test-fail` lines are malformed.
- Quoted values accept `\"` and `\\` as their only escapes.
- `ok`, `tier0-ok` and `error` are mutually exclusive, and so are `test-pass` and `test-fail`.

### Multi-module cases

Exactly one `.una` file under the case's `src/` carries the directive block; the other files MUST NOT contain directives. In a multi-module case, `error` expectations name the file: `error <code> at <path>:<line>:<col>`, where `<path>` is relative to the case directory (for example `src/a/b.una` or `una.toml`), MUST exist, and MUST NOT contain `:`. Single-module cases keep `at <line>:<col>`.

### Discovery

Every directory directly under `tests/conformance/` MUST be one of the areas above; files there (such as `README.md`) are not cases. Inside an area, every `.una` file is a single-module case and every directory containing `una.toml` or `ita.una` is a multi-module case; any other entry is malformed. A run that discovers no case fails. Entries whose names start with `.` are ignored everywhere: at the root, inside areas and among the files of a multi-module case (ADR 0013, D6).

### Pending cases

`-- pending: M<n>` declares that the case is expected to fail until milestone `M<n>` of `plan/milestones-v0.1.md`, with `n` from 1 to 10. The runner validates every directive of a pending case and runs it like any other case, then classifies it:

| `pending` | Outcome matches all expectations | Result |
| --- | --- | --- |
| absent | yes | pass |
| absent | no | fail |
| present | no | pending: listed in the runner summary, does not fail the run |
| present | yes | fail, with the message `pending case passes; remove -- pending: M<n>` |

An expectation the compiler cannot evaluate yet (for example `run-output` before `una run` exists) does not match. A milestone is complete only when no case names it in `pending`, and no case may be pending at release (M10).

## 3. Solver tests

Tests that reach tier 1 require Z3 at the version in the repository's `verification.lock`. If the binary is missing or mismatched, they fail with U5003 and a message explaining how to install the pinned version. They are never skipped.

## 4. Determinism tests

- Running `una check --json` twice on each example produces byte-identical output.
- Running with `RAYON_NUM_THREADS=1` and with the default thread count produces identical output.

## 5. Coverage expectations for v0.1

- Every code in `spec/13 §4` except U8001 is produced by at least one conformance case.
- Every typing rule in `spec/10` and evaluation rule in `spec/11` is exercised by at least one positive case.

## 6. Examples and the guide

- `crates/una-cli/tests/examples.rs` checks every package of `examples/`: canonical form, tier 1 with every obligation proven, every declared example passing, no trivial `ensures` in the report, and the output of `una run` for a package with `[run]`. It also checks that the examples together contain every construct of the language list of charter §5, one source pattern per construct, and that each pattern tells programs apart (ADR 0035 rule 1).
- `crates/una-cli/tests/guide.rs` checks each example's `README.md` (#586, #827). Its code is `sh` blocks, whose commands run, and `text` blocks, each of which names what it shows in its fence's info string:
  - `una="ARGS"`: what `una --color never ARGS` prints on standard output in a copy of the package. The command must exit with 0. The commands run in the README's order, in one copy shared by all of them, after `una run .`, `una build .` and `una check .`. The `una="run ."` block is the whole output of `una run .`, and of the native executable.
  - `file="PATH"`: an excerpt of that file of the package, a relative path inside it with no `..`.
  - `after="mistakes/N"`, beside one of the two: what the package shows after the change that the README's `N`th item under `## Common mistakes` describes. Any other value fails. The change is the files of `tests/fixtures/examples/NAME/mistakes/N/`, outside the shipped examples, each replacing one of the package's files, laid over a fresh copy of it. `una --color never check .` must reject the changed copy and print nothing on standard output. After a change, `una="check ."` is the only command: such a block is an excerpt of what it prints on standard error, anchored as an output is. A `file="PATH"` block is an excerpt of a file the change replaces. A change holds files under `src/` only, no hidden file or symbolic link, and each is the package's current file with one change, whose size, the lines it replaces and the lines it puts in their place, is pinned in `crates/una-cli/tests/guide.rs` (`CHANGES`), so an edit of the example, next to the change too, makes the change fail rather than go stale. Every common mistake has its change and a block of what the check prints after it, each block sits under the mistake it names, and every directory of changes is one its README shows.

  An excerpt is checked with one rule. With trailing spaces removed from every line, its lines are consecutive lines of the file or the output, except that a line that is exactly `...` stands for any number of lines, none included. It is the only wildcard. An excerpt of an output is anchored: it starts at the output's first line and ends at its last, unless it starts or ends with `...`. An excerpt of a file may be anywhere in it. An excerpt with no line but blank ones and `...` fails. So does a `text` block without a marker, with another attribute, or with both `una` and `file`, and a fence of tildes or an indented fence, which the check would not read. Every mismatch is listed together. The website shows the blocks unchanged: its renderer reads only the language and its own attributes.
- `crates/una-cli/tests/guide.rs` checks every block of `docs/guide/una-for-agents.md`: a `una` block is a whole module that checks clean at tier 1, or yields only the code its first line names (`-- fails with U####`); blocks after file paths form a package that checks clean; a `json` block parses and names a schema the CLI prints; each `una` line of a `sh` block exits with 0, or with the code of its `# exit N`, from the repository root or the directory of its `# in <dir>` (ADR 0035 rule 2). It also checks that every `una` block of `README.md` is copied from a checked file or checks clean.
- `scripts/no-libm.sh` checks that no part of Una reaches the C math library (#1092, `docs/release-gates.md`, the `build` gate). It reads release artifacts only, built for the check, since a test build unifies the features of dev-dependencies in and is not what ships: `scripts/local-ci.sh`, after its tests, runs `cargo build --locked --release -p una-cli -p una-rt`, then `scripts/no-libm-build.sh`, which links with `target/release/una` every runnable program of `tests/conformance/runtime` that `tests/native/unsupported.txt` does not list, chosen as the native differential chooses them, into a fresh `target/no-libm`, then `scripts/no-libm.sh` on `target/release/una`, `target/release/libuna_rt.a` and those executables. It asserts that the release graph enables no `std` feature of num-bigint, num-integer and num-traits, and fails on any symbol of the C math library that one of the artifacts leaves undefined, with no exception. The conformance package `tests/conformance/runtime/no_libm_run` gives it an executable that uses big `Int`, `F64`, text, lists and maps.

## 7. Memory budgets in native tests

Native tests link the debug build of `una-rt`. In debug builds only, the runtime reuses a freed slot only after 65,536 newer ones have been freed, so that a use after free is a fault and not a read of another object (#201). A program that frees many values therefore holds a fixed extra amount once about 65,000 values have been freed. One example is a `for` loop, which frees its state tuple at each iteration.

Measured on a `List[Int]` built by `append` in a `for` loop (#878):

- **Debug builds:** a plateau of about 14 MB above the list itself. The smallest budgets were 2 MiB at 10,000 elements, 16 MiB at 100,000 and 24 MiB at 1,000,000.
- **Release builds:** linear, about 12 bytes per element at the peak. That is the list's 8-byte words, plus the old buffer while it grows.

The loop makes no copy at any size: `a_list_built_by_append_in_a_loop_is_never_copied_at_any_size` pins it through the debug copy counter.

The rule follows: a native test that sets `--max-memory` uses a budget above this plateau, or compares its outcome with `una run` rather than with a budget near the program's own size. To test a budget close to a program's data, build the data without garbage per iteration, for example with `together for`.
