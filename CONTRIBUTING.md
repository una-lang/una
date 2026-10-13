# Contributing to Una

Thank you for your interest in Una. Reports from people who use the language are the most useful contribution right now: a program the compiler gets wrong, a diagnostic that misleads, a guide that is unclear.

## Reporting

Open an [issue](https://github.com/una-lang/una/issues/new/choose) and pick the template that fits:

- **Bug report**: the compiler crashes, rejects a valid program, or prints the wrong output.
- **Soundness report**: `una check` accepts a program that breaks a guarantee Una claims to prove (types, authority, information flow, contracts). These are the most valuable reports.
- **Question**: anything about the language, the tools or the documentation.

Include the smallest program that shows the problem, the command you ran, what you expected, and the output of `una --version`. If a report could hurt users before it is fixed, such as a way for a program to escape its grants at run time, follow [`SECURITY.md`](SECURITY.md) instead of opening a public issue.

## How work flows

1. **An issue first.** Every change starts as an issue, so its scope is agreed before code is written. One issue per branch and per pull request.
2. **The specification first.** [`docs/spec/`](docs/spec/) defines the language, and when the code and the specification disagree, the specification wins. A change to the language starts as an architecture decision record: copy [`docs/adr/0000-template.md`](docs/adr/0000-template.md) to `docs/adr/NNNN-short-title.md` with status `Proposed`. It is implemented only once it is `Accepted`, and the specification is updated in the same change as the implementation. Accepted records are not edited; later changes are appended as a dated amendment.
3. **Tests first.** Write the tests that encode the rule before implementing it. A language rule gets conformance cases in [`tests/conformance/`](tests/conformance/), positive and negative, in the format of [`docs/compiler/testing.md`](docs/compiler/testing.md). A fix gets a regression test that fails before the fix.
4. **Implement**, following [`docs/compiler/conventions.md`](docs/compiler/conventions.md).
5. **Validate locally**, then open the pull request.

Branches are named `dev/<issue>-<topic>`, for example `dev/6-integration-rule`. Every commit references its issue (`Refs #6`), and the pull request closes it (`Closes #6`, one keyword per issue). Each pull request adds a line to the `## Unreleased` section of [`CHANGELOG.md`](CHANGELOG.md).

## Validation

Validation runs locally. A pull request is merged after `scripts/local-ci.sh` passes on its final commit:

```bash
scripts/install-z3.sh "$HOME/.local/z3" && export PATH="$HOME/.local/z3/bin:$PATH"
scripts/local-ci.sh           # solver check, formatting, clippy, every test, the release archive
scripts/local-ci.sh --linux   # the same steps in a Linux container
```

A pull request that changes `scripts/local-ci.sh`, its Docker image under `scripts/local-ci/`, or a script that one of its steps calls also needs `scripts/local-ci.sh --linux-arm64` to pass on its final commit, since a tool the image lacks fails only there (`docs/releases/validation.md` §1). Each run ends with `PASS`, or with `FAIL` and the step that failed. One step builds the website with `scripts/site.sh` when [mdBook](https://rust-lang.github.io/mdBook/) is installed at the version the script pins, and says it skipped it otherwise; the release gates require it. Install it with `cargo install mdbook --version 0.5.4 --locked`. The site is generated from the repository's documents into `target/site`; open `target/site/index.html` to read it. During development, the usual commands are:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo insta review   # review each snapshot change; never accept one blindly
```

Tests that reach the solver need Z3 at the exact version pinned in `verification.lock`. Without it they fail with U5003 rather than skip.

Releases are checked the same way: `scripts/release-gates.sh` runs every gate on one commit from a fresh clone, and each release's record is in [`docs/releases/`](docs/releases/).

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
