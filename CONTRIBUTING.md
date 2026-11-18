# Contributing to Una

Thank you for your interest in Una. Reports from people who use the language are the most useful contribution right now: a program the compiler gets wrong, a diagnostic that misleads, a guide that is unclear.

## Reporting

Open an [issue](https://github.com/una-lang/una/issues/new/choose) and pick the template that fits:

- **Bug report**: the compiler crashes, rejects a valid program, or prints the wrong output.
- **Soundness report**: `una check` accepts a program that breaks a guarantee Una claims to prove (types, authority, information flow, contracts). These are the most valuable reports.
- **Question**: anything about the language, the tools or the documentation.

Include the smallest program that shows the problem, the command you ran, what you expected, and the output of `una --version`. If a report could hurt users before it is fixed, such as a way for a program to escape its grants at run time, follow [`SECURITY.md`](SECURITY.md) instead of opening a public issue.

## What is in this repository

The public tree holds the compiler's sources (`crates/`), the specification (`docs/spec/`), the guides (`docs/guide/`), the standard library listing (`docs/std/`), the compiler's design notes (`docs/compiler/`), the test suites (`tests/` and the integration tests of each crate), the examples, the validation records of each release (`docs/validation/`), the security documents, the formal development (`formal/`), the fuzz targets and the pinned supply-chain inputs. The maintainers keep the rest, which is not in this tree: the architecture decision records, the roadmap and plan, the review records, and the scripts for validation, release and the website. Where a document names one of those, it says it is kept by the maintainers.

## How work flows

1. **An issue first.** Every change starts as an issue, so its scope is agreed before code is written. One issue per branch and per pull request.
2. **The specification first.** [`docs/spec/`](docs/spec/) defines the language, and when the code and the specification disagree, the specification wins. A change to the language is first agreed in the issue, as a proposal that the maintainers record as an architecture decision; the specification is updated in the same change as the implementation.
3. **Implement**, following [`docs/compiler/conventions.md`](docs/compiler/conventions.md). Write the tests that encode a rule before implementing it: a language rule gets conformance cases in [`tests/conformance/`](tests/conformance/), positive and negative, in the format of [`docs/compiler/testing.md`](docs/compiler/testing.md), and a fix gets a regression test that fails before it.
4. **Check locally**, then open the pull request.

Branches are named `dev/<issue>-<topic>`, for example `dev/6-integration-rule`. Every commit references its issue (`Refs #6`), and the pull request closes it (`Closes #6`, one keyword per issue). Each pull request adds one changelog fragment, `changelog.d/<issue>-<slug>.md`: a single line that is one bullet, `- **What changed** (#<issue>, …). Details.`, and never edits [`CHANGELOG.md`](CHANGELOG.md) itself. Two pull requests then never change the same lines. The maintainers assemble the fragments at each release.

## Building and checking

You need the Rust toolchain pinned in `rust-toolchain.toml` and the Z3 library that `una` links; the README's [Build from source](README.md#build-from-source) explains both (`scripts/z3-library.sh` builds the library from the source `supply-chain/z3.lock` pins). Then:

```bash
eval "$(scripts/z3-library.sh --export)"
cargo build --release -p una-cli -p una-rt
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo nextest run --workspace --locked   # the test suites; `cargo test --workspace --locked` runs them too
```

These commands are known to pass on macOS arm64, the platform of the maintainers' validation; on other platforms `cargo clippy` may report warnings that are not yet fixed. The public tree has no continuous-integration workflow.

`una` uses no `z3` on `PATH`. Without `UNA_Z3_LIB` the build makes a `una` with no solver, and a check at tier 1 fails with U5003. A pull request is merged after the maintainers' own validation on its final commit; a release is tagged when every gate passes on one commit, or when the maintainer accepts it in the record, whose verdict line says so. Each release's record is in [`docs/validation/`](docs/validation/) and states which gates ran and on which commit, as [`docs/release-gates.md`](docs/release-gates.md) describes.

## Rules for the code

The full list is in [`docs/compiler/conventions.md`](docs/compiler/conventions.md). The most important:

- No `unsafe`, except in the documented boundary module of the native runtime.
- No `unwrap`, `expect` or `panic!` in library crates, except for internal invariants marked `// INVARIANT: <reason>`. Problems a user can cause are diagnostics with a registered code.
- Library crates never print; only `una-cli` writes to the terminal.
- Output is deterministic: no hash-map iteration, timestamps or randomness in anything a user sees.
- Every public item has a doc comment, and every diagnostic code has an entry in the specification and an `una explain` text.
- Choose the algorithm with the best average-case complexity, and state it.

## Writing

Documentation and messages are in English, with American spelling. Normative text uses MUST, MUST NOT, SHOULD and MAY. A diagnostic message is one sentence with a lowercase start and no trailing period, and it names the construct: `capability @audit is not in scope`.

## Conduct

Everyone who takes part in the project follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree that your contributions are licensed under the Apache License, Version 2.0, the license of the project.
