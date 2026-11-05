# Testing strategy

Status: Normative for the reference implementation.

## 1. Layers

| Layer | Location | Purpose |
| --- | --- | --- |
| Unit | `crates/*/src/**` | Small functions: lexer states, unification, formula normalization |
| Conformance | `tests/conformance/` | One file per spec rule, positive and negative |
| Snapshot | `crates/*/tests/snapshots/` (insta) | Full JSON diagnostics, Core IR dumps, SMT queries, reports |
| Property | `crates/una-fmt/tests/` | Formatter idempotence and parse round-trip over every `.una` file in the repository |
| Serve scripts | `tests/serve/` | Connections, bytes, paced reads and closes at virtual milliseconds through the real server engine, with the bytes each connection received (ADR 0123 rule 21; `crates/una-http/tests/serve_scripts.rs`) |
| Serve runs | `tests/serve_runs/` | A server program of edition 0.4, checked at tier 1, run in the interpreter over the real server engine fed by a script of connections, with the bytes each connection received and the program's output and trace (ADR 0123 rules 1 to 4, 18 and 21; `crates/una-cli/tests/serve_runs.rs`) |
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

### Independence from the directories above

A single file takes the edition of the nearest `ita.una` or `una.toml` above it, however far up (`spec/30`, #1175), so a stray manifest above the checkout, above `CARGO_TARGET_DIR` or above the system's temporary directory would change the edition of every single-file case below it. The tests do not depend on what lies above them (#1560), with one exception. The runner and the tests that read the corpus or the examples in place (`cli.rs`, `core_symbols.rs`, `demo_scripts.rs`, `runtime.rs`, `test_declaration.rs`, `verification.rs`, `widening_repairs.rs`) read a copy of them below a seal, a `una.toml` of edition `0.1`, and the tests that write single files under a temporary directory (`explain_examples.rs`, `native.rs`, `recovery.rs`, `serve_grants.rs`, `unknown_names.rs`) seal that directory first (`crates/una-cli/tests/common/hermetic.rs`); `ancestor_manifests.rs` plants a manifest above both kinds of fixture and checks that every call site keeps its seal. The exception is `named_file_edition.rs`'s test of a single file with no manifest above it, which cannot be sealed by design: it fails with a message naming the manifest when one lies above the temporary directory. The repository holds no seal of its own, since a `una.toml` at its root would also be the package root that `una lsp` finds for its single files.

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

(ADR 0124.) From step 1 of that record, the solver is the Z3 built into `una` itself: a `una` built without it is a development build whose tier-1 tests fail with U5003, "this development build has no solver built in; run scripts/build-z3.sh". The library is built once per pin and toolchain by `scripts/build-z3.sh` from the tarball that `supply-chain/z3.lock` pins and the patches it lists, into a cache outside every worktree and target directory: `UNA_Z3_CACHE`, else `~/.cache/una/z3`, under a key that is the SHA-256 over the tarball's digest, each patch's digest in the lock's order, and the toolchain (the C++ compiler's and cmake's version lines and the machine), so a cold worktree never pays the 10 to 20 minutes of the build again on one machine. Each entry holds `lib/libz3shell.a`, `bin/z3` (the standalone front end from the same objects, for the `solver-identity` gate), `build-record.txt`, `configure.log` and `library.sha256`, the library's digest, written last, so that a half-written entry never counts. `scripts/z3-library.sh` computes the key (`scripts/build-z3.sh --key`), checks the library against `library.sha256`, refuses and rebuilds an entry whose bytes changed, and prints the library's path, which `scripts/local-ci.sh` and the release gates name to every cargo step by `UNA_Z3_LIB`; `cargo` alone needs `UNA_Z3_LIB` set to that path (`crates/una-z3/build.rs`). The Docker legs keep the cache in a named volume per architecture, `una-local-ci-z3-cache<suffix>`, mounted at `/opt/z3-cache` and kept across fresh target volumes. Until the mirror release of the tarball exists, `UNA_Z3_SOURCE` names a local copy of it.

A fresh checkout is ready for the tests with one command, which builds the library when the cache has none for this machine's toolchain (a new C++ compiler or cmake changes the key, so an update of the toolchain builds it once more) and exports its path:

```bash
eval "$(scripts/z3-library.sh --export)"   # UNA_Z3_LIB=~/.cache/una/z3/<key>/lib/libz3shell.a
cargo test --workspace --locked
```

Without `UNA_Z3_LIB`, `cargo` builds a development `una` with no solver and the build of `una-z3` warns so; about 120 tests then fail with U5003. Changing `UNA_Z3_LIB` rebuilds `una-z3` and what depends on it.

A test harness that calls `una_cli` in its own process starts its solver children from the `una` of the same build (`target/<profile>/una`, under the feature `test-hooks`), a file it does not run from. The first launch of a newly linked executable costs the operating system's one-time work before the program's first instruction, more than a second for the debug `una` on macOS, so the harness runs its `una` once (`una --version`) before its first child, outside the child's 2-second handshake window, which starts at the spawn (#1278). A shipped `una` re-executes the file it already runs from and never pays that cost in the window.

(ADR 0124 rule 7, Question 6, #1364.) The outer-sandbox tests (`crates/una-sys/tests/solver_confinement.rs` and `crates/una-cli/tests/outer_sandbox.rs`, macOS) run `sandbox-exec` themselves. Under a sandbox that refuses a nested one, as some harnesses apply to the whole test process, they cannot test the outer sandbox, and they fail rather than pass. `UNA_SKIP_NESTED_SANDBOX_TESTS=1` makes them skip there instead, each saying so on stderr; the variable changes nothing where `sandbox-exec` works. No gate sets it: `scripts/local-ci.sh` and the release gates run the tests unsandboxed, and the `hunt` gate sandboxes `una` runs, never the test process.

(ADR 0124 step 2, #1207.) The tests run development builds, which embed no runtime: `una build` reads the runtime from `UNA_RT_LIB` or beside `una`, as `crates/una-cli/tests/common/native_build.rs` arranges, and a runtime file older than `una` still fails there with #203's hint. A release `una` links the runtime built into it, which `scripts/release.sh` embeds (`UNA_EMBED_RUNTIME`, `UNA_EMBED_RUNTIME_SHA256`) and `scripts/check-release.sh` checks. So that the tests reach that path, the feature `test-hooks` lets a development `una` stand a file in for the embedded runtime: `UNA_TEST_EMBEDDED_RUNTIME` names it and `UNA_TEST_EMBEDDED_RUNTIME_SHA256` the digest it must have (`crate::runtime_archive`). `crates/una-cli/tests/native.rs` builds with it, with `UNA_RT_LIB` naming a file that is no runtime, and requires the executable of a development build from the same runtime file; and it requires U8001, exit code 3, with no executable, for a stand-in that is missing or differs from its digest by one bit. No shipped `una` has the hook: `scripts/check-release.sh` refuses the marker of `test-hooks`.

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
- `scripts/no-libm.sh` checks that no part of Una reaches the C math library (#1092, `docs/release-gates.md`, the `build` gate). It reads release artifacts only, built for the check, since a test build unifies the features of dev-dependencies in and is not what ships: `scripts/local-ci.sh`, after its tests, runs `cargo build --locked --release -p una-cli -p una-rt`, then `scripts/no-libm-build.sh`, which links with `target/release/una` every runnable program of `tests/conformance/runtime` that `tests/native/unsupported.txt` does not list, chosen as the native differential chooses them, into a fresh `target/no-libm`, then `scripts/no-libm.sh` on `target/release/una`, `target/release/libuna_rt.a` and those executables. It asserts that the release graph holds none of num-bigint, num-integer and num-traits, which Una's own big integer replaced (#1093), and fails on any symbol of the C math library that the runtime or an executable leaves undefined, with no exception. The release `una`, which holds the Z3 solver, may also import the names of `scripts/no-libm-una.txt`, the solver's own, each tied to the Z3 library `UNA_Z3_LIB` names (#1277); `una` built without the solver, which `scripts/no-libm-build.sh` builds into `target/no-solver`, is strict. `scripts/no-libm-test.sh` and `scripts/load-commands-test.sh` run the canaries of this check and of `scripts/load-commands.sh` before them: small artifacts built with the pinned `rustc` that must fail each check, in `una`, in the runtime and in an executable. The conformance package `tests/conformance/runtime/no_libm_run` gives it an executable that uses big `Int`, `F64`, text, lists and maps.

## 7. Memory budgets in native tests

Native tests link the debug build of `una-rt`. In debug builds only, the runtime reuses a freed slot only after 65,536 newer ones have been freed, so that a use after free is a fault and not a read of another object (#201). A program that frees many values therefore holds a fixed extra amount once about 65,000 values have been freed. One example is a `for` loop, which frees its state tuple at each iteration.

The debug memory budget credits part of that amount back (#1506). A heap credits the freed slots of its slot table past the most objects it ever held at once, but only the table capacity above twice the most slots a release build's table ever needed for that heap (its peak, and each import's reservation), or 4 slots. No growth of a release build's table goes past that bound, so the credited slots are never capacity a release build's table may hold. The free list and the rest of the table stay charged, and the credit of all heaps together is at most a quarter of the budget. What is guaranteed is only that the debug build is not charged less than a release build for the slots it credits. The two builds' tables can still differ, in either direction, by how their growth fell, so a debug run may need a different budget than the same run in a release build. A check reports U6003 before the allocator's hard ceiling of twice the budget, except after one allocation of more than three quarters of the budget between two checks.

Measured on a `List[Int]` built by `append` in a `for` loop (#878):

- **Debug builds:** a plateau of about 14 MB above the list itself. The smallest budgets were 2 MiB at 10,000 elements, 16 MiB at 100,000 and 24 MiB at 1,000,000.
- **Release builds:** linear, about 12 bytes per element at the peak. That is the list's 8-byte words, plus the old buffer while it grows.

The loop makes no copy at any size: `a_list_built_by_append_in_a_loop_is_never_copied_at_any_size` pins it through the debug copy counter.

The rule follows: a native test that sets `--max-memory` uses a budget above this plateau, or compares its outcome with `una run` rather than with a budget near the program's own size. To test a budget close to a program's data, build the data without garbage per iteration, for example with `together for`.
